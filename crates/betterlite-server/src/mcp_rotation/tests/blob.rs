#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::secret_codec::MAX_ENCODED_BYTES;

use super::super::blob::{
    RotationBlob, decode_json_column, decode_legacy_base64_text, decode_mcp_rotation_blob,
};
use super::super::error::RotationCodecError;
use super::support::json;

#[test]
fn rotation_blob_prefers_json() {
    for (text, expected) in [
        (r#"{"client_id":"value"}"#, json(r#"{"client_id":"value"}"#)),
        ("[1,2]", json("[1,2]")),
        ("null", JsonValue::Null),
        ("17", json("17")),
        (r#""aGVsbG8=""#, json(r#""aGVsbG8=""#)),
    ] {
        let decoded = decode_mcp_rotation_blob(text.as_bytes()).unwrap();
        assert!(decoded == RotationBlob::Json(expected), "{text}");
    }
}

#[test]
fn rotation_blob_falls_back_to_legacy_base64() {
    let encoded = URL_SAFE.encode("legacy-secret");
    let decoded = decode_mcp_rotation_blob(encoded.as_bytes()).unwrap();
    assert!(decoded == RotationBlob::Legacy("legacy-secret".to_owned()));
    let padded = decode_mcp_rotation_blob(b"aGVsbG8=").unwrap();
    assert!(padded == RotationBlob::Legacy("hello".to_owned()));
}

#[test]
fn rotation_blob_rejects_bad_base64() {
    let result = decode_mcp_rotation_blob(b"not json");
    assert!(matches!(result, Err(RotationCodecError::Secret(_))));
}

#[test]
fn rotation_blob_rejects_invalid_utf8() {
    let result = decode_mcp_rotation_blob(&[b'{', 0xff, b'}']);
    assert!(matches!(result, Err(RotationCodecError::Utf8(_))));
}

#[test]
fn rotation_blob_rejects_oversize_and_depth() {
    let oversized = vec![b' '; 0x0010_0001];
    assert!(matches!(
        decode_mcp_rotation_blob(&oversized),
        Err(RotationCodecError::Blob(_))
    ));
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(
        decode_mcp_rotation_blob(deep.as_bytes()),
        Err(RotationCodecError::Blob(_))
    ));
}

#[test]
fn legacy_base64_matches_cpython() {
    for (text, expected) in [
        ("", ""),
        ("***", ""),
        ("aGVsbG8=", "hello"),
        ("aGVs bG8=", "hello"),
        ("AA==", "\0"),
        ("4b-_", "\u{1fff}"),
    ] {
        assert_eq!(
            decode_legacy_base64_text(text).unwrap(),
            expected,
            "{text:?}"
        );
    }
    for rejected in ["aGVsbG8", "AA", "A", "AA=", "__8=", "\u{e9}", "not json"] {
        assert!(
            decode_legacy_base64_text(rejected).is_err(),
            "{rejected:?} must not decode"
        );
    }
    let at_limit = " ".repeat(MAX_ENCODED_BYTES);
    assert_eq!(decode_legacy_base64_text(&at_limit).unwrap(), "");
    let over_limit = " ".repeat(MAX_ENCODED_BYTES + 1);
    assert!(decode_legacy_base64_text(&over_limit).is_err());
}

#[test]
fn json_column_decoding() {
    assert_eq!(decode_json_column(None).unwrap(), None);
    assert_eq!(
        decode_json_column(Some(r#"{"a":1}"#)).unwrap(),
        Some(json(r#"{"a":1}"#))
    );
    assert!(decode_json_column(Some("{bad")).is_err());
}

#[test]
fn replays_the_mcp_rotation_blob_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mcp_rotation_blob");
    let entries: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("the corpus directory")
        .map(|entry| entry.expect("a corpus entry").path())
        .collect();
    assert!(!entries.is_empty(), "the corpus has no entries");
    for path in entries {
        let bytes = fs::read(path).expect("a corpus file");
        let _decoded = decode_mcp_rotation_blob(&bytes);
        let _column = decode_json_column(std::str::from_utf8(&bytes).ok());
    }
}
