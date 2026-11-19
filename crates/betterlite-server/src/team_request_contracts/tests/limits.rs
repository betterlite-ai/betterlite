use std::fs::{read, read_dir};
use std::path::{Path, PathBuf};

use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{MAX_JSON_BYTES, MAX_JSON_DEPTH};

use super::support::parse;
use crate::team_request_contracts::{DecodeError, TypedValue, decode_team_request};

fn nested_arrays(levels: usize) -> String {
    let mut text = String::from(r#"{"a": "#);
    text.push_str(&"[".repeat(levels));
    text.push('1');
    text.push_str(&"]".repeat(levels));
    text.push('}');
    text
}

fn seed(directory: &Path, name: &str) -> bool {
    decode_team_request(&read(directory.join(name)).unwrap()).is_ok()
}

#[test]
fn request_byte_limit() {
    let base = br#"{"team_id": "t"}"#;
    let mut accepted = vec![b' '; MAX_JSON_BYTES];
    accepted[..base.len()].copy_from_slice(base);
    let decoded = decode_team_request(&accepted).unwrap();
    assert_eq!(decoded.json, parse(r#"{"team_id": "t"}"#));
    let rejected = vec![b' '; MAX_JSON_BYTES + 1];
    let error = decode_team_request(&rejected).unwrap_err();
    assert!(matches!(
        &error,
        DecodeError::Bounded(PrivateJsonError::TooLarge(..))
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "the JSON payload is {} bytes, over the {MAX_JSON_BYTES}-byte limit",
            MAX_JSON_BYTES + 1
        )
    );
    decode_team_request(&[]).unwrap_err();
}

#[test]
fn request_json_depth_limit() {
    let at_limit = nested_arrays(MAX_JSON_DEPTH - 1);
    decode_team_request(at_limit.as_bytes()).unwrap();
    let over = nested_arrays(MAX_JSON_DEPTH);
    let error = decode_team_request(over.as_bytes()).unwrap_err();
    assert!(matches!(
        &error,
        DecodeError::Bounded(PrivateJsonError::TooDeep(..))
    ));
    assert_eq!(
        error.to_string(),
        format!("the JSON payload nests deeper than {MAX_JSON_DEPTH} levels")
    );
    let brackets_in_text = format!(r#"{{"a": "{}"}}"#, "[".repeat(200));
    decode_team_request(brackets_in_text.as_bytes()).unwrap();
    let accepted_object = format!(r#"{{"a": {}}}"#, nested_arrays(1));
    decode_team_request(accepted_object.as_bytes()).unwrap();
}

#[test]
fn decode_rejects_malformed_and_non_objects() {
    let truncated = decode_team_request(br#"{"team_id": "#).unwrap_err();
    assert!(matches!(
        &truncated,
        DecodeError::Bounded(PrivateJsonError::Decode(_))
    ));
    assert!(
        truncated
            .to_string()
            .starts_with("the JSON payload cannot be decoded")
    );
    let invalid_utf8 = decode_team_request(b"{\"team_id\": \"\xff\"}").unwrap_err();
    assert!(matches!(
        &invalid_utf8,
        DecodeError::Bounded(PrivateJsonError::Decode(_))
    ));
    let array = decode_team_request(b"[1]").unwrap_err();
    assert!(matches!(&array, DecodeError::NotAnObject));
    assert_eq!(
        array.to_string(),
        "the team request document is not a JSON object"
    );
    assert!(matches!(
        decode_team_request(b"null").unwrap_err(),
        DecodeError::NotAnObject
    ));
    let duplicate = decode_team_request(br#"{"team_id": "first", "team_id": "second"}"#).unwrap();
    assert_eq!(duplicate.json, parse(r#"{"team_id": "second"}"#));
    let precise = decode_team_request(br#"{"team_id": "t", "max_budget": 10.50}"#).unwrap();
    assert_eq!(
        precise.json,
        parse(r#"{"team_id": "t", "max_budget": 10.50}"#)
    );
}

#[test]
fn replays_the_team_request_contracts_corpus() {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/team_request_contracts");
    let seeds: Vec<PathBuf> = read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(seeds.len() >= 6);
    let outcomes: Vec<Result<TypedValue, DecodeError>> = seeds
        .iter()
        .map(|path| decode_team_request(&read(path).unwrap()))
        .collect();
    assert!(outcomes.iter().any(Result::is_ok));
    assert!(outcomes.iter().any(Result::is_err));
    assert!(seed(&directory, "valid-team-create.json"));
    assert!(seed(&directory, "valid-bulk-members.json"));
    assert!(seed(&directory, "duplicate-team-id.json"));
    assert!(!seed(&directory, "truncated.json"));
    assert!(!seed(&directory, "malformed.json"));
    assert!(!seed(&directory, "not-an-object.json"));
    assert!(!seed(&directory, "too-deep.json"));
}
