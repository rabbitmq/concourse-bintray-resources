extern crate bintray;
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use bintray::client::{BintrayClient, BintrayError};
use bintray::repository::{self, Repository};
use bintray::utils;
use clap::{App, Arg};
use std::env;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufReader, BufRead, Read, Write};
use std::path::Path;

const VERSION: Option<&'static str> = option_env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    username: String,
    api_key: String,
    subject: String,
    repository: String,
    repository_type: repository::RepositoryType,
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
    created: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InInput {
    source: Source,
    version: Option<CheckVersion>,
    params: Option<serde_json::Value> // TODO: Be more restrictive.
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
    private: Option<bool>,
    business_unit: Option<StringOrFile>,
    desc: Option<StringOrFile>,
    labels: Option<StringVecOrFile>,
    gpg_sign_metadata: Option<bool>,
    gpg_sign_files: Option<bool>,
    gpg_use_owner_key: Option<bool>,

    yum_metadata_depth: Option<u64>
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

    let matches = App::new("Concourse resource for Bintray repositories")
        .version(VERSION.unwrap_or("DEV"))
        .author("The RabbitMQ Team")
        .about("Allows to create Bintray repositories from a Concourse job")
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

    let mut repo = Repository::new(&input.source.subject,
                                   &input.source.repository);

    match repo.get(&client) {
        Ok(()) => { }
        Err(e) => { error_out(&e) }
    }

    // Print the result as JSON on stdout.
    let result = get_check_result(&repo);
    match serde_json::to_string_pretty(&result) {
        Ok(output) => { println!("{}", output); }
        Err(e)     => { error_out(&BintrayError::Json(e)); }
    };
}

fn get_check_result(repo: &Repository) -> Vec<CheckVersion> {
    match version_for_concourse(repo) {
        Some(version) => vec![version],
        None          => vec![],
    }
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

    let mut repo = Repository::new(&input.source.subject,
                                   &input.source.repository);

    match repo.get(&client) {
        Ok(()) => { }
        Err(e) => { error_out(&e) }
    }

    if repo.type_ != input.source.repository_type {
        error_out(&io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(concat!(
                        "The repository type from the confiuration ({}) ",
                        "doesn't match the existing repository type ({})"),
                        input.source.repository_type, repo.type_)));
    }

    // Print the result as JSON on stdout.
    let result = get_out_result(&repo);
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

    let client = BintrayClient::new(
        Some(input.source.username),
        Some(input.source.api_key));

    let mut repo = Repository::new(&input.source.subject,
                                   &input.source.repository);

    let exists = match repo.exists(&client) {
        Ok(exists) => exists,
        Err(e)     => error_out(&e),
    };

    if !exists {
        repo.type_ = input.source.repository_type;
    } else if repo.type_ != input.source.repository_type {
        error_out(&io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(concat!(
                        "The repository type from the confiuration ({}) ",
                        "doesn't match the existing repository type ({})"),
                        input.source.repository_type, repo.type_)));
    }

    let initial_repo = repo.clone();

    // Create or update repository properties with input params.
    repo.private = input.params.private.unwrap_or(repo.private);
    repo.business_unit = input.params.business_unit
        .map_or(repo.business_unit, |v| Some(from_string_or_file(&v)));
    repo.desc = input.params.desc
        .map_or(repo.desc, |v| Some(from_string_or_file(&v)));
    repo.labels = input.params.labels
        .map_or(repo.labels, |v| Some(from_string_vec_or_file(&v)))
        .map(|mut v| { v.sort(); v });
    repo.gpg_sign_metadata =
        input.params.gpg_sign_metadata.unwrap_or(repo.gpg_sign_metadata);
    repo.gpg_sign_files =
        input.params.gpg_sign_files.unwrap_or(repo.gpg_sign_files);
    repo.gpg_use_owner_key =
        input.params.gpg_use_owner_key.unwrap_or(repo.gpg_use_owner_key);
    if input.params.yum_metadata_depth.is_some() {
        repo.yum_metadata_depth = input.params.yum_metadata_depth;
    }

    if repo != initial_repo {
        let error_out_closure = |e| -> Result<(), ()> { error_out(&e); };

        let _ = match exists {
            true  => repo.update(&client).or_else(error_out_closure),
            false => repo.create(&client).or_else(error_out_closure),
        };

        let _ = repo.get(&client);
    }

    // Print the result as JSON on stdout.
    let result = get_out_result(&repo);
    match serde_json::to_string_pretty(&result) {
        Ok(output) => { println!("{}", output); }
        Err(e)     => { error_out(&BintrayError::Json(e)); }
    };
}

fn get_out_result(repo: &Repository) -> OutResult {
    let mut metadata = vec![];
    metadata.push(OutMetadata {
        name: String::from("Type"), value: repo.type_.to_string().clone()
    });
    repo.desc.as_ref().and_then(|desc| {
        metadata.push(OutMetadata {
            name: String::from("Desc."), value: desc.clone()
        });
        Some(())
    });
    OutResult {
        version: version_for_concourse(repo).expect("Repository not created yet"),
        metadata: metadata,
    }
}

// -------------------------------------------------------------------
// Internal functions.
// -------------------------------------------------------------------

fn version_for_concourse(repo: &Repository) -> Option<CheckVersion> {
    repo.created.as_ref().map(|v| CheckVersion { created: v.clone() })
}

fn error_out<E: std::error::Error>(error: &E) -> ! {
    let _ = writeln!(&mut std::io::stderr(), "\n\x1b[31m{}\x1b[0m", error);
    std::process::exit(1);
}

fn error_out_with_filename(filename: &str, error: BintrayError) -> ! {
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
