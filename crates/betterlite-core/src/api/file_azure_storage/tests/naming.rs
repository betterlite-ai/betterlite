#![cfg(test)]

use std::time::Duration;

use crate::api::file_azure_storage::naming::{generated_file_name, naming_strategy};
use crate::api::file_storage_contracts::storage::FileNamingStrategy;

use super::support::{FIXED_ID, FixedIds};

const TIMESTAMP_MILLIS: u64 = 1_234;

fn timestamp_name(filename: &str) -> String {
    generated_file_name(
        filename,
        FileNamingStrategy::Timestamp,
        Duration::from_millis(TIMESTAMP_MILLIS),
        &FixedIds::new(FIXED_ID),
    )
}

fn original_name(filename: &str) -> String {
    generated_file_name(
        filename,
        FileNamingStrategy::OriginalFilename,
        Duration::from_millis(TIMESTAMP_MILLIS),
        &FixedIds::new(FIXED_ID),
    )
}

fn uuid_name(filename: &str) -> String {
    generated_file_name(
        filename,
        FileNamingStrategy::Uuid,
        Duration::from_millis(TIMESTAMP_MILLIS),
        &FixedIds::new(FIXED_ID),
    )
}

#[test]
fn azure_filename_strategies() {
    assert_eq!(original_name("dir/a b.TXT"), "a%20b.TXT");
    assert_eq!(timestamp_name("dir/a b.TXT"), "1234.TXT");
    assert_eq!(
        timestamp_name_at("dir/a b.TXT", Duration::from_millis(1_001)),
        "1000.TXT"
    );
    assert_eq!(uuid_name("dir/a b.TXT"), format!("{FIXED_ID}.TXT"));
}

// Python's int(time.time() * 1000) truncates once, so 1.001 seconds counts as 1000 milliseconds
fn timestamp_name_at(filename: &str, now: Duration) -> String {
    generated_file_name(
        filename,
        FileNamingStrategy::Timestamp,
        now,
        &FixedIds::new(FIXED_ID),
    )
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

#[test]
fn invalid_basenames_fall_back_to_file() {
    for filename in ["", ".", "..", "a\0b.txt", "dir/"] {
        assert_eq!(original_name(filename), "file", "{filename:?}");
        assert_eq!(timestamp_name(filename), "1234", "{filename:?}");
        assert_eq!(uuid_name(filename), FIXED_ID, "{filename:?}");
    }
}

#[test]
fn extensions_come_from_the_last_dot() {
    assert_eq!(timestamp_name(".env"), "1234");
    assert_eq!(timestamp_name("a.tar.gz"), "1234.gz");
    assert_eq!(timestamp_name("a."), "1234");
    assert_eq!(timestamp_name("a..b"), "1234.b");
    assert_eq!(timestamp_name("a"), "1234");
    assert_eq!(timestamp_name(".hidden.tar"), "1234.tar");
}

// a traversal sequence only ever reaches the basename, so it can never become an extension
#[test]
fn traversal_cannot_become_an_extension() {
    assert_eq!(timestamp_name("a.jsonl/../../etc/cron.d/x"), "1234");
    assert_eq!(timestamp_name("../../etc/passwd"), "1234");
    assert_eq!(original_name("../../etc/passwd"), "passwd");
}

#[test]
fn original_names_are_percent_quoted() {
    assert_eq!(original_name("dir/é.txt"), "%C3%A9.txt");
    assert_eq!(original_name("dir/a\\b.txt"), "b.txt");
    assert_eq!(original_name("dir/a%20b.txt"), "a%2520b.txt");
    assert_eq!(original_name("a+b.txt"), "a%2Bb.txt");
    assert_eq!(original_name("a_b-c~d.txt"), "a_b-c~d.txt");
}
