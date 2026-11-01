#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str;

use betterlite_logging::opik::config_file::read_config_section;
use rstest::rstest;

#[test]
fn config_file_fails_on_line_before_any_header() {
    let error = read_config_section("key = value\n[opik]\nother = 1\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("before any section header"));
    assert!(!text.contains("value"));
}

#[test]
fn config_file_fails_without_delimiter() {
    let error = read_config_section("[opik]\nbroken line with no delimiter\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("no = or :"));
    assert!(!text.contains("broken line with no delimiter"));
}

#[test]
fn config_file_fails_on_empty_key() {
    let error = read_config_section("[opik]\n = value\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("empty key"));
    assert!(!text.contains("value"));
}

#[test]
fn config_file_fails_on_repeated_section() {
    let error = read_config_section("[opik]\na = 1\n[opik]\nb = 2\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("repeated"));
    assert!(!text.contains('1') && !text.contains('2'));
}

#[test]
fn config_file_fails_on_key_repeated_across_default_blocks() {
    let error = read_config_section("[DEFAULT]\nshared = 1\n[DEFAULT]\nshared = 2\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("DEFAULT"));
    assert!(text.contains("shared"));
    assert!(!text.contains('1') && !text.contains('2'));
}

#[test]
fn config_file_fails_on_nesting_past_ten() {
    let error = read_config_section("[opik]\na = %(a)s\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("nests past"));
    assert!(!text.contains('%'));
}

#[test]
fn config_file_resolves_a_chain_nested_exactly_ten_deep() {
    let text = "[opik]\n\
                a0 = %(a1)s\n\
                a1 = %(a2)s\n\
                a2 = %(a3)s\n\
                a3 = %(a4)s\n\
                a4 = %(a5)s\n\
                a5 = %(a6)s\n\
                a6 = %(a7)s\n\
                a7 = %(a8)s\n\
                a8 = %(a9)s\n\
                a9 = %(a10)s\n\
                a10 = end\n";
    let values = read_config_section(text).expect("ten levels of nesting is not too deep");
    assert_eq!(values.get("a0"), Some(&String::from("end")));
}

#[test]
fn config_file_fails_on_missing_name() {
    let error = read_config_section("[opik]\na = %(missing)s\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("unknown option"));
    assert!(!text.contains("missing)s"));
}

#[test]
fn config_file_fails_on_trailing_percent() {
    let error = read_config_section("[opik]\na = value%\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("malformed"));
    assert!(!text.contains("value"));
}

#[test]
fn config_file_double_percent_escapes_to_one_percent() {
    let values = read_config_section("[opik]\na = 100%% full\n").expect("valid configuration");
    assert_eq!(values.get("a"), Some(&String::from("100% full")));
}

#[test]
fn config_file_fails_on_bad_percent_in_default_with_other_section() {
    let error = read_config_section("[DEFAULT]\nbroken = 50%\n[opik]\nx = 1\n").unwrap_err();
    let text = error.to_string();
    assert!(text.contains("opik"));
    assert!(text.contains("broken"));
    assert!(!text.contains("50"));
}

#[test]
fn config_file_succeeds_on_bad_percent_in_default_without_other_section() {
    let values = read_config_section("[DEFAULT]\nbroken = 50%\n").expect("no section to build");
    assert!(values.is_empty());
}

#[test]
fn config_file_reads_header_with_trailing_text() {
    let values =
        read_config_section("[opik] extra text here\nkey = value\n").expect("valid configuration");
    assert_eq!(values.get("key"), Some(&String::from("value")));
}

#[test]
fn config_file_deeper_indented_bracket_continues_value() {
    let values = read_config_section("[opik]\nnote = first\n  [x]\n").expect("valid configuration");
    assert_eq!(values.get("note"), Some(&String::from("first\n[x]")));
}

#[test]
fn config_file_accepts_lone_cr_line_ending() {
    let values = read_config_section("[opik]\rkey = value\r").expect("valid configuration");
    assert_eq!(values.get("key"), Some(&String::from("value")));
}

#[test]
fn config_file_treats_uppercase_section_as_different_from_opik() {
    let values = read_config_section("[OPIK]\nkey = value\n").expect("no opik section to build");
    assert!(values.is_empty());
}

#[rstest]
fn config_file_replays_the_corpus(#[files("fuzz/corpus/opik_config_file/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("reads the corpus file");
    if let Ok(text) = str::from_utf8(&bytes) {
        drop(read_config_section(text));
    }
}
