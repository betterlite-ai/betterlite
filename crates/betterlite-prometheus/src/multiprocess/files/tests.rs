#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::multiprocess::MultiprocessError;
use crate::multiprocess::files::{DbEntry, decode_db_file, encode_db_file};

fn entry(key: &str, value: f64, timestamp: f64) -> DbEntry {
    DbEntry {
        key: key.to_owned(),
        value,
        timestamp,
    }
}

#[test]
fn round_trips_entries() {
    let entries = vec![
        entry(
            r#"["requests", "requests_total", {"model": "a"}, "requests"]"#,
            3.5,
            1234.5,
        ),
        entry(r#"["queue", "queue_length", {}, "queue"]"#, -2.0, 0.0),
    ];
    let bytes = encode_db_file(&entries);
    assert_eq!(bytes.len(), 0x0001_0000);
    assert_eq!(decode_db_file(&bytes).unwrap(), entries);
}

#[test]
fn writes_a_fresh_page() {
    let bytes = encode_db_file(&[]);
    assert_eq!(bytes.len(), 0x0001_0000);
    assert_eq!(&bytes[..8], [8_u8, 0, 0, 0, 0, 0, 0, 0].as_slice());
    assert!(bytes[8..].iter().all(|byte| *byte == 0));
    assert!(decode_db_file(&bytes).unwrap().is_empty());
}

#[test]
fn grows_the_file_while_entries_do_not_fit() {
    let key = "k".repeat(70000);
    let entries = vec![entry(&key, 1.0, 2.0)];
    let bytes = encode_db_file(&entries);
    assert_eq!(bytes.len(), 0x0002_0000);
    assert_eq!(decode_db_file(&bytes).unwrap(), entries);
}

#[test]
fn keeps_the_page_when_the_entries_fill_it_exactly() {
    let key = "k".repeat(65504);
    let entries = vec![entry(&key, 1.0, 2.0)];
    let bytes = encode_db_file(&entries);
    assert_eq!(bytes.len(), 0x0001_0000);
    assert_eq!(decode_db_file(&bytes).unwrap(), entries);
}

#[rstest]
#[case(0, 4)]
#[case(3, 1)]
#[case(4, 8)]
#[case(5, 7)]
#[case(8, 4)]
#[case(12, 8)]
#[case(30, 6)]
fn pads_key_lengths_around_the_alignment(#[case] key_length: usize, #[case] padding: usize) {
    assert_eq!(super::entry_padding(key_length), padding);
    let entries = vec![entry("k".repeat(key_length).as_str(), 1.0, 2.0)];
    assert_eq!(decode_db_file(&encode_db_file(&entries)).unwrap(), entries);
}

#[test]
fn reads_a_file_with_no_entries() {
    let bytes = [8_u8, 0, 0, 0, 0, 0, 0, 0];
    assert!(decode_db_file(&bytes).unwrap().is_empty());
}

#[test]
fn reads_a_key_that_ends_at_the_used_length() {
    let mut bytes = vec![12_u8, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&4_i32.to_le_bytes());
    bytes.extend_from_slice(b"abcd");
    bytes.extend_from_slice(&[b' '; 8]);
    bytes.extend_from_slice(&1.5_f64.to_le_bytes());
    bytes.extend_from_slice(&0.0_f64.to_le_bytes());
    let entries = decode_db_file(&bytes).unwrap();
    assert_eq!(entries, vec![entry("abcd", 1.5, 0.0)]);
}

#[test]
fn fails_on_a_file_shorter_than_the_header() {
    let error = decode_db_file(&[0_u8, 0, 0]).unwrap_err();
    assert!(matches!(error, MultiprocessError::Truncated { offset: 0 }));
}

#[test]
fn fails_on_a_length_field_that_is_cut() {
    let mut bytes = vec![12_u8, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&[0_u8, 0]);
    let error = decode_db_file(&bytes).unwrap_err();
    assert!(matches!(error, MultiprocessError::Truncated { offset: 8 }));
}

#[test]
fn fails_on_a_negative_key_length() {
    let mut bytes = vec![12_u8, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&(-1_i32).to_le_bytes());
    let error = decode_db_file(&bytes).unwrap_err();
    assert!(matches!(error, MultiprocessError::Corrupted));
    assert_eq!(
        error.to_string(),
        "Read beyond file size detected, file is corrupted."
    );
}

#[test]
fn fails_on_a_key_that_ends_past_used() {
    let mut bytes = vec![12_u8, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&10_i32.to_le_bytes());
    bytes.extend_from_slice(b"short");
    let error = decode_db_file(&bytes).unwrap_err();
    assert!(matches!(error, MultiprocessError::Corrupted));
    assert_eq!(
        error.to_string(),
        "Read beyond file size detected, file is corrupted."
    );
}

#[test]
fn fails_on_a_key_that_is_not_utf8() {
    let mut bytes = vec![32_u8, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&2_i32.to_le_bytes());
    bytes.extend_from_slice(&[0xff_u8, 0xfe]);
    bytes.extend_from_slice(b"  ");
    bytes.extend_from_slice(&1.0_f64.to_le_bytes());
    bytes.extend_from_slice(&0.0_f64.to_le_bytes());
    let error = decode_db_file(&bytes).unwrap_err();
    assert!(matches!(
        error,
        MultiprocessError::NonUtf8Key { offset: 12 }
    ));
    assert_eq!(error.to_string(), "the key at offset 12 is not UTF-8");
}

#[test]
fn fails_when_the_used_length_runs_past_the_bytes() {
    let mut bytes = vec![64_u8, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.extend_from_slice(b"k");
    bytes.extend_from_slice(b"   ");
    bytes.extend_from_slice(&1.0_f64.to_le_bytes());
    bytes.extend_from_slice(&0.0_f64.to_le_bytes());
    let error = decode_db_file(&bytes).unwrap_err();
    assert!(matches!(error, MultiprocessError::Truncated { offset: 32 }));
}

#[rstest]
fn replays_the_db_file_decode_corpus(#[files("fuzz/corpus/db_file_decode/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus seed");
    let name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .expect("a seed name")
        .to_owned();
    if name == "two-entries" {
        let entries = decode_db_file(&bytes).expect("a valid seed");
        assert_eq!(entries.len(), 2);
        assert_eq!(encode_db_file(&entries), bytes);
        return;
    }
    let error = decode_db_file(&bytes).unwrap_err();
    if name == "empty" {
        assert!(matches!(error, MultiprocessError::Truncated { offset: 0 }));
        return;
    }
    if name == "key-past-used" {
        assert!(matches!(error, MultiprocessError::Corrupted));
        return;
    }
    if name == "non-utf8-key" {
        assert!(matches!(
            error,
            MultiprocessError::NonUtf8Key { offset: 12 }
        ));
        return;
    }
    assert!(matches!(error, MultiprocessError::Truncated { offset: 32 }));
}
