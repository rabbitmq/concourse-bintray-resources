extern crate bintray;
extern crate clap;
extern crate env_logger;
extern crate glob;
#[macro_use] extern crate log;
extern crate regex;
#[macro_use] extern crate serde_derive;
extern crate serde_json;

use bintray::client::{BintrayClient, BintrayError};
use bintray::repository::Repository;
use bintray::package::{Package, PackageMaturity};
use bintray::version::Version;
use bintray::file::Content;
use bintray::utils;
use clap::{App, Arg};
use glob::glob;
use regex::Regex;
use std::borrow::Borrow;
use std::env;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufReader, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::{thread, time};

const VERSION: Option<&'static str> = option_env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    username: String,
    api_key: String,
    subject: String,
    repository: String,
    package: String,
    gpg_passphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckInput {
    source: Source,
    version: Option<CheckVersion>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckVersion {
    version: String,
    #[serde(skip_serializing_if="Option::is_none")]
    updated: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InInput {
    source: Source,
    version: Option<CheckVersion>,
    params: Option<InParams>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InParams {
    local_path: Option<StringOrFile>,
    remote_path: Option<StringOrFile>,
    filter: Option<StringVecOrFile>,
    version: StringOrFile,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutInput {
    source: Source,
    params: OutParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutParams {
    local_path: Option<StringOrFile>,
    remote_path: Option<StringOrFile>,
    filter: Option<StringVecOrFile>,
    version: StringOrFile,

    package_props: Option<PackagePropsOutParams>,
    version_props: Option<VersionPropsOutParams>,

    publish: Option<bool>,
    #[serde(rename = "override")]
    override_: Option<bool>,

    debian_architecture: Option<StringVecOrFile>,
    debian_distribution: Option<StringVecOrFile>,
    debian_component: Option<StringVecOrFile>,

    show_in_download_list: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackagePropsOutParams {
    desc: Option<StringOrFile>,
    labels: Option<StringVecOrFile>,
    public_download_numbers: Option<bool>,
    public_stats: Option<bool>,
    maturity: Option<StringOrFile>,

    licenses: Option<StringVecOrFile>,
    custom_licenses: Option<StringVecOrFile>,

    website_url: Option<StringOrFile>,
    issue_tracker_url: Option<StringOrFile>,
    vcs_url: Option<StringOrFile>,
    github_repo: Option<StringOrFile>,
    github_release_notes_file: Option<StringOrFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionPropsOutParams {
    desc: Option<StringOrFile>,
    released: Option<StringOrFile>,

    vcs_tag: Option<StringOrFile>,
    github_release_notes_file: Option<StringOrFile>,
    github_use_tag_release_notes: Option<bool>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct OutResult {
    version: CheckVersion,
    metadata: Vec<OutMetadata>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct OutMetadata {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields,untagged)]
enum StringOrFile {
    FromString(String),
    FromFile(FromFile),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields,untagged)]
enum StringVecOrFile {
    FromStringVec(Vec<String>),
    FromString(String),
    FromFile(FromFile),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FromFile {
    from_file: String,
}

fn main() {
    /* Initialize logger. */
    env_logger::init().unwrap();

    let matches = App::new("Concourse resource for Bintray packages")
        .version(VERSION.unwrap_or("DEV"))
        .author("The RabbitMQ Team")
        .about("Allows to publish packages to Bintray from a Concourse job")
        .arg(Arg::with_name("script")
             .help("Name of the Concourse resource script to act as")
             .short("s")
             .long("script")
             .value_name("SCRIPT")
             .possible_values(&["check", "in", "out"]))
        .arg(Arg::with_name("WORKING DIR")
             .help("Source or destination directory for the 'in' and 'out' scripts"))
        .get_matches();

    /* Look at the program's name to determine what we should do. */
    let program_name = env::current_exe().ok().as_ref()
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .map(String::from)
        .expect("Failed to determine program name");

    match matches.value_of("WORKING DIR") {
        Some(path) => {
            env::set_current_dir(&path)
                .unwrap_or_else(|e| error_out(&BintrayError::from(e)));
        }
        None => { }
    }

    match matches.value_of("script").unwrap_or(program_name.as_ref()) {
        "check" => check(),
        "out"   => out(),
        "in"    => in_(),
        _ => {
            let _ = writeln!(
                &mut std::io::stderr(),
                "\x1b[31mProgram name unrecognized: {:?}\x1b[0m", program_name);
            std::process::exit(64);
        }
    }
}

// -------------------------------------------------------------------
// Resource `check` operation.
// -------------------------------------------------------------------

fn check() {
    /* Read and parse JSON from stdin. */
    let mut input = String::new();
    {
        let stdin = io::stdin();
        let mut stdin_handle = stdin.lock();
        stdin_handle.read_to_string(&mut input)
            .unwrap_or_else(|e| error_out(&BintrayError::from(e)));
        info!("Input:\n{}", utils::prettify_json(&input));
    }

    let input: CheckInput = match serde_json::from_str(&input) {
        Ok(i)  => { i }
        Err(e) => { error_out(&BintrayError::Json(e)); }
    };

    let client = BintrayClient::new(
        Some(input.source.username),
        Some(input.source.api_key));

    let mut package = Package::new(&input.source.subject,
                                   &input.source.repository,
                                   &input.source.package);

    match package.get(false, &client) {
        Ok(()) => { }
        Err(e) => { error_out(&e) }
    }

    // Print the result as JSON on stdout.
    let result = get_check_result(&package, input.version, &client);
    match serde_json::to_string_pretty(&result) {
        Ok(output) => { println!("{}", output); }
        Err(e)     => { error_out(&BintrayError::Json(e)); }
    };
}

fn get_check_result(package: &Package,
                    version: Option<CheckVersion>,
                    client: &BintrayClient)
    -> Vec<CheckVersion>
{
    let versions = match version {
        Some(version) => {
            package.get_versions_starting_at(
                &Some(version.version),
                Some(client))
        }
        None => {
            match package.get_latest_version(Some(client)) {
                Some(version) => vec![version],
                None          => vec![]
            }
        }
    };

    versions
        .iter()
        .map(version_for_concourse)
        .collect()
}

// -------------------------------------------------------------------
// Resource `in` operation.
// -------------------------------------------------------------------

fn in_() {
    /* Read and parse JSON from stdin. */
    let mut input = String::new();
    {
        let stdin = io::stdin();
        let mut stdin_handle = stdin.lock();
        stdin_handle.read_to_string(&mut input)
            .unwrap_or_else(|e| error_out(&BintrayError::from(e)));
        info!("Input:\n{}", utils::prettify_json(&input));
    }

    let input: InInput = match serde_json::from_str(&input) {
        Ok(i)  => { i }
        Err(e) => { error_out(&BintrayError::Json(e)); }
    };

    let client = BintrayClient::new(
        Some(input.source.username),
        Some(input.source.api_key));

    let mut package = Package::new(&input.source.subject,
                                   &input.source.repository,
                                   &input.source.package);

    match package.get(false, &client) {
        Ok(()) => { }
        Err(e) => { error_out(&e) }
    }

    // Create or update version properties with input params.
    let version_string = match input.version {
        Some(version) => version.version,
        None => {
            package.get_latest_version(Some(&client))
                .unwrap_or_else(|| error_out(&io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("The package {} has no version",
                            package))))
                .version
        }
    };
    let mut version = Version::new(&input.source.subject,
                                   &input.source.repository,
                                   &input.source.package,
                                   &version_string);

    match version.get(false, &client) {
        Ok(()) => { }
        Err(e) => { error_out(&e) }
    }

    // TODO: Download files.

    // Print the result as JSON on stdout.
    let result = get_out_result(&version);
    match serde_json::to_string_pretty(&result) {
        Ok(output) => { println!("{}", output); }
        Err(e)     => { error_out(&BintrayError::Json(e)); }
    };
}

// -------------------------------------------------------------------
// Resource `out` operation.
// -------------------------------------------------------------------

fn out() {
    /* Read and parse JSON from stdin. */
    let mut input = String::new();
    {
        let stdin = io::stdin();
        let mut stdin_handle = stdin.lock();
        stdin_handle.read_to_string(&mut input)
            .unwrap_or_else(|e| error_out(&BintrayError::from(e)));
        info!("Input:\n{}", utils::prettify_json(&input));
    }

    let input: OutInput = match serde_json::from_str(&input) {
        Ok(i)  => { i }
        Err(e) => { error_out(&BintrayError::Json(e)); }
    };

    // Find all files to include in the package/version.
    let local_path = input.params.local_path
        .map_or(String::new(), |v| from_string_or_file(&v));
    let remote_path = input.params.remote_path
        .map_or(String::new(), |v| from_string_or_file(&v));
    let _ = writeln!(&mut std::io::stderr(),
        "\x1b[32mFrom:\x1b[0m {}\n  \x1b[32mTo:\x1b[0m {}\n",
        local_path, remote_path);

    if ! local_path.is_empty() {
        env::set_current_dir(&local_path)
            .unwrap_or_else(|e| error_out(&BintrayError::from(e)));
    }

    let files = find_files(input.params.filter);
    let version_string = determine_version(input.params.version, &files);

    let client = BintrayClient::new(
        Some(input.source.username.clone()),
        Some(input.source.api_key.clone()));

    let mut repo = Repository::new(&input.source.subject,
                                   &input.source.repository);

    match repo.exists(&client) {
        Ok(true) => {}
        Ok(false) => {
            error_out(&io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("The repository {} doesn't exist",
                            repo)));
        }
        Err(e) => { error_out(&e); }
    };

    // Create or update package properties with input params.
    let _ = update_package(input.params.package_props,
                           &input.source,
                           &client);

    // Create or update version properties with input params.
    let version = update_version(input.params.version_props,
                                 &input.source,
                                 &version_string,
                                 &client);

    let mut old_files = version.list_files(true, &client)
        .unwrap_or_else(|e| error_out(&e));

    // Upload all files.
    let publish = match input.params.publish {
        Some(v) => v,
        None    => true,
    };
    let override_ = match input.params.override_ {
        Some(v) => v,
        None    => true,
    };
    let gpg_passphrase = input.source.gpg_passphrase
        .as_ref().map(String::as_str);
    let debian_architecture = input.params.debian_architecture
        .map(|v| from_string_vec_or_file(&v))
        .unwrap_or(vec![]);
    let debian_distribution = input.params.debian_distribution
        .map(|v| from_string_vec_or_file(&v))
        .unwrap_or(vec![]);
    let debian_component = input.params.debian_component
        .map(|v| from_string_vec_or_file(&v))
        .unwrap_or(vec![]);

    let files = files.iter()
        .map(|filename| upload_file(&filename,
                                    &remote_path,
                                    publish,
                                    override_,
                                    gpg_passphrase,
                                    &debian_architecture,
                                    &debian_distribution,
                                    &debian_component,
                                    &version,
                                    &client))
        .collect::<Vec<Content>>();
    let _ = writeln!(&mut std::io::stderr(), "");

    // Remove files which shouldn't be part of the version anymore.
    old_files.retain(|ref remote| {
        !files.iter().any(|ref local| {
            let mut abs_remote = PathBuf::from("/");
            abs_remote.push(&remote.path);
            let mut abs_local = PathBuf::from("/");
            abs_local.push(&local.path);
            abs_remote == abs_local
        })
    });
    if old_files.len() > 0 {
        let _ = old_files.iter().fold((), |_, ref f| {
            remove_file(&f, &client);
        });
        let _ = writeln!(&mut std::io::stderr(), "");
    }

    if publish {
        let _ = writeln!(&mut std::io::stderr(),
        "\x1b[32mMark version as published...\x1b[0m");
        let mut remaining_files = files.len();
        while remaining_files > 0 {
            remaining_files = version
                .publish_content(Some(-1), false, &client)
                .unwrap_or_else(|e| error_out(&e));

            if remaining_files > 0 {
                thread::sleep(time::Duration::from_secs(30));
            }
        }
    }

    let show_in_download_list = match input.params.show_in_download_list {
        Some(v) => v,
        None    => true,
    };
    if publish && show_in_download_list {
        let _ = writeln!(&mut std::io::stderr(),
        "\x1b[32mShow in download list...\x1b[0m");
        let _ = files.iter()
            .map(|ref f| {
                loop {
                    // Even if the "publish version" request above
                    // returned there is no more files to publish for
                    // the version, files may not be published yet at
                    // the package level. Therefore we might get a Bad
                    // Request error from the API (NotFound from the
                    // crate). If this happens, we retry 30 seconds
                    // later.
                    match f.show_in_download_list(true, &client) {
                        Ok(_) => { break; }
                        Err(BintrayError::Io(ref e))
                        if e.kind() == io::ErrorKind::NotFound => {
                            thread::sleep(time::Duration::from_secs(30));
                        }
                        Err(e) => { error_out(&e); }
                    }
                }
            })
            .collect::<Vec<_>>();
    }

    // Print the result as JSON on stdout.
    let result = get_out_result(&version);
    match serde_json::to_string_pretty(&result) {
        Ok(output) => { println!("{}", output); }
        Err(e)     => { error_out(&BintrayError::Json(e)); }
    };
}

fn find_files(filter: Option<StringVecOrFile>) -> Vec<PathBuf> {
    let globs = filter.map_or(
        vec![String::from("**")],
        |v| from_string_vec_or_file(&v));

    let mut result = vec![];
    result.extend(globs.iter()
        .flat_map(|pattern| {
            glob(pattern).unwrap_or_else(|e| { error_out(&e); })
        })
        .map(|glob_result| {
            glob_result.unwrap_or_else(|e| { error_out(&e); })
        }));

    let _ = writeln!(&mut std::io::stderr(),
        "\x1b[32mFiles:\x1b[0m");
    for path in result.iter() {
        let _ = writeln!(&mut std::io::stderr(),
            "    {}", path.to_str().unwrap_or(""));
    }
    let _ = writeln!(&mut std::io::stderr(), "");

    result
}

fn determine_version(version: StringOrFile, files: &Vec<PathBuf>) -> String {
    // We first need to get the version string. It's available from one
    // of the following sources:
    //  * a regex against files which are part of the package/version;
    //  * a text file.
    let version_string = match version {
        StringOrFile::FromString(regex) => {
            let re = Regex::new(&regex)
                .unwrap_or_else(|e| error_out(&e));
            files.iter()
                .fold(None,
                      |acc, ref pathbuf| -> Option<String> {
                          acc.or_else(|| capture_version(&re, &pathbuf))
                      })
                .unwrap_or_else(
                    || error_out(
                        &io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "Failed to determine version from file names")))
        }
        StringOrFile::FromFile(_) => {
            from_string_or_file(&version)
        }
    };

    let _ = writeln!(&mut std::io::stderr(),
        "\x1b[32mVersion:\x1b[0m\n    {}\n", version_string);

    version_string
}

fn capture_version(re: &Regex, pathbuf: &PathBuf) -> Option<String> {
    pathbuf.to_str().and_then(
        |file|
        match re.captures(file) {
            Some(caps) => {
                caps.name("version").or(caps.get(1))
                    .map(|v| String::from(v.as_str()))
            }
            None => None,
        })
}

fn update_package(props: Option<PackagePropsOutParams>,
                  source: &Source,
                  client: &BintrayClient) -> Package
{
    // Create or update package properties with input params.
    let mut package = Package::new(&source.subject,
                                   &source.repository,
                                   &source.package);

    let exists = match package.exists(client) {
        Ok(exists) => exists,
        Err(e)     => error_out(&e),
    };

    let initial_package = package.clone();

    match props {
        Some(props) => {
            package.desc = props.desc
                .map_or(package.desc, |v| Some(from_string_or_file(&v)));
            package.labels = props.labels
                .map_or(package.labels,
                        |v| Some(from_string_vec_or_file(&v)))
                .map(|mut v| { v.sort(); v });
            package.public_download_numbers = props.public_download_numbers
                .unwrap_or(package.public_download_numbers);
            package.public_stats = props.public_stats
                .unwrap_or(package.public_stats);
            package.maturity = props.maturity
                .map_or(
                    package.maturity,
                    |v| Some(PackageMaturity::from(from_string_or_file(&v))));

            package.licenses = props.licenses
                .map_or(package.licenses,
                        |v| Some(from_string_vec_or_file(&v)))
                .map(|mut v| { v.sort(); v });
            package.custom_licenses = props.custom_licenses
                .map_or(package.custom_licenses,
                        |v| Some(from_string_vec_or_file(&v)))
                .map(|mut v| { v.sort(); v });

            package.website_url = props.website_url
                .map_or(package.website_url,
                        |v| Some(from_string_or_file(&v)));
            package.issue_tracker_url = props.issue_tracker_url
                .map_or(package.issue_tracker_url,
                        |v| Some(from_string_or_file(&v)));
            package.vcs_url = props.vcs_url
                .map_or(package.vcs_url, |v| Some(from_string_or_file(&v)));
            package.github_repo = props.github_repo
                .map_or(package.github_repo,
                        |v| Some(from_string_or_file(&v)));
            package.github_release_notes_file = props.github_release_notes_file
                .map_or(package.github_release_notes_file,
                        |v| Some(from_string_or_file(&v)));
        }
        None => { }
    }

    if !exists || package != initial_package {
        let error_out_closure = |e| -> Result<(), ()> { error_out(&e); };

        let _ = match exists {
            true  => {
                let _ = writeln!(&mut std::io::stderr(),
                "\x1b[32mUpdate package record:\x1b[0m {}", package);
                package.update(client).or_else(error_out_closure)
            }
            false => {
                let _ = writeln!(&mut std::io::stderr(),
                "\x1b[32mCreate package record:\x1b[0m {}", package);
                package.create(client).or_else(error_out_closure)
            },
        };

        let _ = package.get(false, client);
    } else {
        let _ = writeln!(&mut std::io::stderr(),
        "Package record {} up-to-date", package);
    }

    package
}

fn update_version(props: Option<VersionPropsOutParams>,
                  source: &Source,
                  version_string: &str,
                  client: &BintrayClient)
    -> Version
{
    // Create or update package properties with input params.
    let mut version = Version::new(&source.subject,
                                   &source.repository,
                                   &source.package,
                                   version_string);

    let exists = match version.exists(client) {
        Ok(exists) => exists,
        Err(e)     => error_out(&e),
    };

    let initial_version = version.clone();

    match props {
        Some(props) => {
            version.desc = props.desc
                .map_or(version.desc, |v| Some(from_string_or_file(&v)));
            version.released = props.released
                .map_or(version.released, |v| Some(from_string_or_file(&v)));

            version.vcs_tag = props.vcs_tag
                .map_or(version.vcs_tag, |v| Some(from_string_or_file(&v)));
            version.github_release_notes_file = props.github_release_notes_file
                .map_or(version.github_release_notes_file,
                        |v| Some(from_string_or_file(&v)));
            version.github_use_tag_release_notes =
                props.github_use_tag_release_notes
                .or(version.github_use_tag_release_notes);
        }
        None => { }
    }

    if !exists || version != initial_version {
        let error_out_closure = |e| -> Result<(), ()> { error_out(&e); };

        let _ = match exists {
            true  => {
                let _ = writeln!(&mut std::io::stderr(),
                "\x1b[32mCreate version record:\x1b[0m {}", version);
                version.update(client).or_else(error_out_closure)
            }
            false => {
                let _ = writeln!(&mut std::io::stderr(),
                "\x1b[32mCreate version record:\x1b[0m {}", version);
                version.create(client).or_else(error_out_closure)
            }
        };

        let _ = version.get(false, client);
    } else {
        let _ = writeln!(&mut std::io::stderr(),
        "Version record {} up-to-date", version);
    }

    version
}

fn upload_file<T: Borrow<str>>(filename: &PathBuf,
                               remote_path: &str,
                               publish: bool,
                               override_: bool,
                               gpg_passphrase: Option<&str>,
                               debian_architecture: &[T],
                               debian_distribution: &[T],
                               debian_component: &[T],
                               version: &Version,
                               client: &BintrayClient) -> Content
{
    let mut path = PathBuf::from(remote_path);
    path.push(filename);

    let file = Content::new(&version.owner,
                            &version.repository,
                            &version.package,
                            &version.version,
                            &path);

    let _ = writeln!(&mut std::io::stderr(),
    "\x1b[32mUpload file:\x1b[0m {}", file.path.display());

    match file.upload(filename, publish, override_, false,
                      gpg_passphrase,
                      debian_architecture,
                      debian_distribution,
                      debian_component,
                      client) {
        Ok(warning) => log_bintray_warning(warning),
        Err(e)      => error_out(&e),
    };

    file
}

fn remove_file(file: &Content, client: &BintrayClient) {
    let _ = writeln!(&mut std::io::stderr(),
    "\x1b[34mRemove file:\x1b[0m {}", file.path.display());

    match file.remove(client) {
        Ok(warning) => log_bintray_warning(warning),
        Err(e)      => error_out(&e),
    }
}

fn get_out_result(version: &Version) -> OutResult {
    let mut metadata = vec![];
    version.released.as_ref().and_then(|release| {
        metadata.push(OutMetadata {
            name: String::from("Release date"), value: release.clone()
        });
        Some(())
    });
    OutResult {
        version: version_for_concourse(version),
        metadata: metadata,
    }
}

// -------------------------------------------------------------------
// Internal functions.
// -------------------------------------------------------------------

fn version_for_concourse(version: &Version) -> CheckVersion {
    CheckVersion {
        version: version.version.clone(),
        updated: version.updated.clone(),
    }
}

fn log_bintray_warning(warning: Option<String>) {
    warning.and_then(|m| -> Option<()> {
        let _ =
            writeln!(&mut std::io::stderr(), "\n\x1b[33m{}\x1b[0m", m);
        None
    });
}

fn error_out<E: std::error::Error>(error: &E) -> ! {
    let _ = writeln!(&mut std::io::stderr(), "\n\x1b[31m{}\x1b[0m", error);
    std::process::exit(1);
}

fn error_out_with_filename<E: std::error::Error>(filename: &str,
                                                 error: E) -> !
{
    let _ =
        writeln!(&mut std::io::stderr(), "\x1b[31m{}: {}\x1b[0m",
        filename, error);
    std::process::exit(1);
}

fn from_string_or_file(input: &StringOrFile) -> String
{
    match input {
        &StringOrFile::FromString(ref string) =>
            string.clone(),
        &StringOrFile::FromFile(ref fileparams) => {
            let file = File::open(&fileparams.from_file)
                .unwrap_or_else(
                    |e| error_out_with_filename(&fileparams.from_file,
                                                BintrayError::from(e)));
            let mut buf_reader = BufReader::new(file);
            let mut content = String::new();
            buf_reader.read_to_string(&mut content)
                .unwrap_or_else(
                    |e| error_out_with_filename(&fileparams.from_file,
                                                BintrayError::from(e)));
            String::from(content.trim())
        }
    }
}

fn from_string_vec_or_file(input: &StringVecOrFile) -> Vec<String> {
    match input {
        &StringVecOrFile::FromStringVec(ref vec) =>
            vec.clone(),
        &StringVecOrFile::FromString(ref string) =>
            vec![string.clone()],
        &StringVecOrFile::FromFile(ref fileparams) => {
            let file = File::open(&fileparams.from_file)
                .unwrap_or_else(
                    |e| error_out_with_filename(&fileparams.from_file,
                                                BintrayError::from(e)));
            let buf_reader = BufReader::new(file);
            let mut vec = vec![];
            for line in buf_reader.lines() {
                let line = line
                    .unwrap_or_else(
                        |e| error_out_with_filename(&fileparams.from_file,
                                                    BintrayError::from(e)));
                vec.push(String::from(line.trim()));
            }
            vec
        }
    }
}
