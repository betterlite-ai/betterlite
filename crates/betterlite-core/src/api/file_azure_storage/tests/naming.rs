#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use crate::api::file_azure_storage::naming::{generated_file_name, naming_strategy};
use crate::api::file_storage_contracts::storage::FileNamingStrategy;

use super::support::{FIXED_ID, FixedIds};

const TIMESTAMP_MILLIS: u64 = 1_234;
const AT_MILLIS: Duration = Duration::from_millis(TIMESTAMP_MILLIS);
const AT_ONE_SECOND: Duration = Duration::from_millis(1_001);

fn generated_name(filename: &str, strategy: FileNamingStrategy, now: Duration) -> String {
    generated_file_name(filename, strategy, now, &FixedIds::new(FIXED_ID))
}

#[test]
fn azure_filename_strategies() {
    assert_eq!(
        generated_name(
            "dir/a b.TXT",
            FileNamingStrategy::OriginalFilename,
            AT_MILLIS
        ),
        "a%20b.TXT"
    );
    assert_eq!(
        generated_name("dir/a b.TXT", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234.TXT"
    );
    // Python's int(time.time() * 1000) truncates once, so 1.001 seconds counts as 1000 milliseconds
    assert_eq!(
        generated_name("dir/a b.TXT", FileNamingStrategy::Timestamp, AT_ONE_SECOND),
        "1000.TXT"
    );
    assert_eq!(
        generated_name("dir/a b.TXT", FileNamingStrategy::Uuid, AT_MILLIS),
        format!("{FIXED_ID}.TXT")
    );
    assert_eq!(
        generated_name("dir/a b.TXT", naming_strategy("unknown"), AT_MILLIS),
        format!("{FIXED_ID}.TXT")
    );
}

#[test]
fn naming_strategy_reads_litellms_strings() {
    assert_eq!(
        naming_strategy("original_filename"),
        FileNamingStrategy::OriginalFilename
    );
    assert_eq!(naming_strategy("timestamp"), FileNamingStrategy::Timestamp);
    assert_eq!(naming_strategy("uuid"), FileNamingStrategy::Uuid);
    assert_eq!(naming_strategy(""), FileNamingStrategy::Uuid);
    assert_eq!(naming_strategy("UUID"), FileNamingStrategy::Uuid);
}

#[rstest]
#[case::empty("")]
#[case::dot(".")]
#[case::dot_dot("..")]
#[case::nul("a\0b.txt")]
#[case::trailing_slash("dir/")]
fn invalid_basenames_fall_back_to_file(#[case] filename: &str) {
    assert_eq!(
        generated_name(filename, FileNamingStrategy::OriginalFilename, AT_MILLIS),
        "file"
    );
    assert_eq!(
        generated_name(filename, FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234"
    );
    assert_eq!(
        generated_name(filename, FileNamingStrategy::Uuid, AT_MILLIS),
        FIXED_ID
    );
}

#[test]
fn extensions_come_from_the_last_dot() {
    assert_eq!(
        generated_name(".env", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234"
    );
    assert_eq!(
        generated_name("a.tar.gz", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234.gz"
    );
    assert_eq!(
        generated_name("a.", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234"
    );
    assert_eq!(
        generated_name("a..b", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234.b"
    );
    assert_eq!(
        generated_name("a", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234"
    );
    assert_eq!(
        generated_name(".hidden.tar", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234.tar"
    );
}

// a traversal sequence only ever reaches the basename, so it can never become an extension
#[test]
fn traversal_cannot_become_an_extension() {
    assert_eq!(
        generated_name(
            "a.jsonl/../../etc/cron.d/x",
            FileNamingStrategy::Timestamp,
            AT_MILLIS
        ),
        "1234"
    );
    assert_eq!(
        generated_name("../../etc/passwd", FileNamingStrategy::Timestamp, AT_MILLIS),
        "1234"
    );
    assert_eq!(
        generated_name(
            "../../etc/passwd",
            FileNamingStrategy::OriginalFilename,
            AT_MILLIS
        ),
        "passwd"
    );
}

#[test]
fn original_names_are_percent_quoted() {
    assert_eq!(
        generated_name("dir/é.txt", FileNamingStrategy::OriginalFilename, AT_MILLIS),
        "%C3%A9.txt"
    );
    assert_eq!(
        generated_name(
            "dir/a\\b.txt",
            FileNamingStrategy::OriginalFilename,
            AT_MILLIS
        ),
        "b.txt"
    );
    assert_eq!(
        generated_name(
            "dir/a%20b.txt",
            FileNamingStrategy::OriginalFilename,
            AT_MILLIS
        ),
        "a%2520b.txt"
    );
    assert_eq!(
        generated_name("a+b.txt", FileNamingStrategy::OriginalFilename, AT_MILLIS),
        "a%2Bb.txt"
    );
    assert_eq!(
        generated_name(
            "a_b-c~d.txt",
            FileNamingStrategy::OriginalFilename,
            AT_MILLIS
        ),
        "a_b-c~d.txt"
    );
}
