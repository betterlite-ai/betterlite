use std::fs;
use std::io::ErrorKind;
use std::iter;
use std::path::PathBuf;

use rstest::rstest;
use tempfile::TempDir;

use crate::environment_validation::config_args::{MAX_CONFIG_BYTES, read_config_args};
use crate::environment_validation::error::EnvironmentValidationError;

fn config_file(directory: &TempDir, contents: &[u8]) -> PathBuf {
    let path = directory.path().join("config.json");
    fs::write(&path, contents).expect("the temporary directory is writable");
    path
}

fn padded_object(total_bytes: u64) -> Vec<u8> {
    let mut contents = br#"{"a":1}"#.to_vec();
    let padding = usize::try_from(total_bytes)
        .expect("the size fits")
        .saturating_sub(contents.len());
    contents.extend(iter::repeat_n(b' ', padding));
    contents
}

#[test]
fn config_args_returns_the_object_in_file_order_with_raw_values() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = config_file(
        &directory,
        br#"{"zeta": 1, "alpha": {"nested": [1, 2]}, "name": "x", "none": null}"#,
    );
    let args = read_config_args(&path).expect("a valid object reads");
    let keys: Vec<&str> = args.keys().map(String::as_str).collect();
    assert_eq!(keys, ["zeta", "alpha", "name", "none"]);
    assert_eq!(args["zeta"].get(), "1");
    assert_eq!(args["alpha"].get(), r#"{"nested": [1, 2]}"#);
    assert_eq!(args["name"].get(), r#""x""#);
    assert_eq!(args["none"].get(), "null");
}

#[test]
fn config_args_an_empty_object_reads_as_empty() {
    let directory = TempDir::new().expect("a temporary directory");
    let args = read_config_args(&config_file(&directory, b"{}")).expect("an empty object reads");
    assert!(args.is_empty());
}

#[test]
fn config_args_a_missing_file_is_a_read_error_naming_the_path() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = directory.path().join("absent.json");
    let outcome = read_config_args(&path);
    assert!(matches!(
        &outcome,
        Err(EnvironmentValidationError::ReadConfig { path: named, source })
            if *named == path && source.kind() == ErrorKind::NotFound
    ));
}

#[test]
fn config_args_a_directory_is_a_read_error() {
    let directory = TempDir::new().expect("a temporary directory");
    let outcome = read_config_args(directory.path());
    assert!(matches!(
        outcome,
        Err(EnvironmentValidationError::ReadConfig { .. })
    ));
}

#[rstest]
#[case::empty_file(b"".as_slice())]
#[case::not_json(b"not json".as_slice())]
#[case::array(b"[1, 2]".as_slice())]
#[case::string(br#""text""#.as_slice())]
#[case::number(b"7".as_slice())]
#[case::null(b"null".as_slice())]
#[case::truncated(br#"{"a": "#.as_slice())]
#[case::trailing_text(br#"{"a": 1} tail"#.as_slice())]
fn config_args_content_that_is_not_one_object_is_a_parse_error(#[case] contents: &[u8]) {
    let directory = TempDir::new().expect("a temporary directory");
    let path = config_file(&directory, contents);
    let outcome = read_config_args(&path);
    assert!(matches!(
        &outcome,
        Err(EnvironmentValidationError::ParseConfig { path: named, .. }) if *named == path
    ));
}

#[test]
fn config_args_a_file_of_exactly_the_limit_reads() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = config_file(&directory, &padded_object(MAX_CONFIG_BYTES));
    let args = read_config_args(&path).expect("a file at the limit reads");
    assert_eq!(args.len(), 1);
}

#[test]
fn config_args_a_file_one_byte_over_the_limit_is_refused() {
    let directory = TempDir::new().expect("a temporary directory");
    let path = config_file(
        &directory,
        &padded_object(MAX_CONFIG_BYTES.saturating_add(1)),
    );
    let outcome = read_config_args(&path);
    assert!(matches!(
        &outcome,
        Err(EnvironmentValidationError::ConfigTooLarge { path: named, limit })
            if *named == path && *limit == MAX_CONFIG_BYTES
    ));
}

#[test]
fn config_args_the_limit_is_ten_mebibytes() {
    assert_eq!(MAX_CONFIG_BYTES, 10_485_760);
}
