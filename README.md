# Overview

This is a concourse resource to work with bintray repositories and packages.

You can download or publish packages using this resource.

# Building

The resource requires `rust` to be installed. You can find more info [here](https://www.rust-lang.org/en-US/install.html)

Run `cargo build` to compile the resource.

The command will put the compiled files into `target/debug` directory with executables for 
each resource.

You can run the executables with `--script` argument to perform `check`, `in` and `out` operations.

You can get more information, by running the executable with `--help` argument.

# Usage

There are two resources in this repository.

## binray-repository resource

This resource is used for creating and updating bintray repositories.

### Source Configuration

- `username` a username to access the bintray API 
- `api_key` an authorization token available from the bintray user profile
- `subject` a bintray organisation name (also called `owner` sometimes)
- `repository` a name of a repository to create or update within the `subject`
- `repository_type` a bintray repository type. Possible types are listed in the [API docs](https://bintray.com/docs/api/#_create_repository)

### Behaviour

#### `check`: Does nothing.

#### `in`: Does nothing. Should not be used

#### `out`: Creates or updates a repository on bintray

##### Parameters

- `private`
- `business_unit`
- `desc`
- `labels`
- `gpg_sign_metadata`
- `gpg_sign_files`
- `gpg_use_owner_key`
- `yum_metadata_depth`

All parameters are optional and described in the [API docs](https://bintray.com/docs/api/#_create_repository)

## bintray-package resource

This resource is used to download and publish packages.

### Source Configuration

- `username`: *Required* a username to access the bintray API 
- `api_key`: *Required* an authorization token available from the bintray user profile
- `subject`: *Required* a bintray organisation name (also called `owner` sometimes)
- `repository`: *Required* a name of a repository to create or update within the `subject`
- `package`: *Required* the name of the package to download/create
- `gpg_passphrase`: *Optional* a [passphrase for keys configured in the repository](https://bintray.com/docs/api/#gpg_signing)
- `version_filter`: *Optional* a globe pattern string, or an array of globe pattern strings to filter package versions. Only useful in the `check` behaviour. Versions returned by the `check` script will match the pattern.

### Behaviour

#### `check`: Lists versions published on bintray chronologically.

The `check` command will return a list of package versions published on bintray. Bintray orders versions chronologically, so the most recent version will be considered "latest".

If `version_filter` is specified, only versions matching the filter glob pattern will be returned.

#### `in`: Downloads a published package.

##### Parameters

- `local_path`: *Optional* the directory where downloaded files are store inside the resource directory.
- `remote_path`: *Optional* the directory from which files are downloaded
- `filter`: *Optional* a glob battern or a list of glob patterns to limit the set of downloaded files 
If no parameters set, the command will download all the package contents and put them into the resource root directory.

#### `out`: Publish a bintray package

Publishes a new version or overrides an existing version of a bintray package.
This script will create a version and upload the package files for this version.

##### Parameters

- `local_path`: *Optional* the directory to get the package files from
- `remote_path`: *Optional* the directory where to upload files
- `filter`: *Optional* a glob pattern or a list of glob patterns to limit the set of files to upload
- `version`: *Required* the regular expression to compute the actual published version from the files. From the first file which matches the regular expression, the first matched group is considered the version to publish.
- `publish`: *Optional* boolean. If the file should be marked as "published" on bintray
- `override`: *Optional* boolean. If the existing files should be overriden by the uploaded.
- `debian_architecture`: *Optional* *only for debian repositories*. Supported debian architecture or a list of architectures.
- `debian_distribution`: *Optional* *only for debian repositories*. Supported debia distribution or a list of distributions.
- `debian_component`: *Optional* *only for debian repositories* A component or a list of components
- `show_in_download_list`: *Optional* boolean. If the file should be listed in the web UI in the downloads section.
- `keep_existing_files`: *Optional* boolean. What to do with files not overriden by the upload.
- `package_props`: *Optional* properties for [create_package](https://bintray.com/docs/api/#_create_package)
    - `desc`: *Optional*
    - `labels`: *Optional*
    - `public_download_numbers`: *Optional*
    - `public_stats`: *Optional*
    - `maturity`: *Optional*
    - `licenses`: *Optional*
    - `custom_licenses`: *Optional*
    - `website_url`: *Optional*
    - `issue_tracker_url`: *Optional*
    - `vcs_url`: *Optional*
    - `github_repo`: *Optional*
    - `github_release_notes_file`: *Optional*
- `version_props`: *Optional* properties for [create_version](https://bintray.com/docs/api/#_create_version)
    - `desc`: *Optional*
    - `released`: *Optional*
    - `vcs_tag`: *Optional*
    - `github_release_notes_file`: *Optional*
    - `github_use_tag_release_notes`: *Optional*

### Examples

