#![cfg(test)]

use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::secret_codec::MAX_ENCODED_BYTES;
use rstest::rstest;

use super::super::blob::decode_mcp_rotation_blob;
use super::super::error::RotationCodecError;
use super::support::json;

#[rstest]
#[case::object(r#"{"client_id":"value"}"#, json(r#"{"client_id":"value"}"#))]
#[case::array("[1,2]", json("[1,2]"))]
#[case::null("null", JsonValue::Null)]
#[case::integer("17", json("17"))]
#[case::text(r#""aGVsbG8=""#, json(r#""aGVsbG8=""#))]
fn rotation_blob_prefers_json(#[case] text: &str, #[case] expected: JsonValue) {
    let decoded = decode_mcp_rotation_blob(text.as_bytes()).unwrap();
    assert!(decoded == expected, "{text}");
}

#[test]
fn rotation_blob_falls_back_to_legacy_base64() {
    let encoded = URL_SAFE.encode("legacy-secret");
    let decoded = decode_mcp_rotation_blob(encoded.as_bytes()).unwrap();
    assert!(decoded == JsonValue::String("legacy-secret".to_owned()));
    let padded = decode_mcp_rotation_blob(b"aGVsbG8=").unwrap();
    assert!(padded == JsonValue::String("hello".to_owned()));
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

#[rstest]
#[case::empty("", "")]
#[case::stray_bits("***", "")]
#[case::padded("aGVsbG8=", "hello")]
#[case::whitespace("aGVs bG8=", "hello")]
#[case::null_byte("AA==", "\0")]
#[case::url_safe("4b-_", "\u{1fff}")]
fn legacy_base64_matches_cpython(#[case] text: &str, #[case] expected: &str) {
    let decoded = decode_mcp_rotation_blob(text.as_bytes()).unwrap();
    assert!(
        decoded == JsonValue::String(expected.to_owned()),
        "{text:?}"
    );
}

#[rstest]
#[case::unpadded("aGVsbG8")]
#[case::one_byte("AA")]
#[case::single_char("A")]
#[case::short_padding("AA=")]
#[case::non_zero_padding("__8=")]
#[case::non_ascii("\u{e9}")]
#[case::not_base64("not json")]
fn legacy_base64_rejects_like_cpython(#[case] rejected: &str) {
    assert!(
        decode_mcp_rotation_blob(rejected.as_bytes()).is_err(),
        "{rejected:?} must not decode"
    );
}

#[test]
fn legacy_base64_size_limit_is_enforced() {
    let at_limit = " ".repeat(MAX_ENCODED_BYTES);
    let decoded = decode_mcp_rotation_blob(at_limit.as_bytes()).unwrap();
    assert!(decoded == JsonValue::String(String::new()));
    let over_limit = " ".repeat(MAX_ENCODED_BYTES + 1);
    assert!(decode_mcp_rotation_blob(over_limit.as_bytes()).is_err());
}

#[rstest]
#[case::deep_nesting("deep-nesting.json")]
#[case::invalid_utf8("invalid-utf8.bin")]
#[case::legacy_base64("legacy-base64.txt")]
#[case::malformed_json("malformed-json.json")]
#[case::secret_map("secret-map.json")]
#[case::user_env_vars("user-env-vars.json")]
fn replays_the_mcp_rotation_blob_corpus(#[case] name: &str) {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/mcp_rotation_blob");
    let entries = fs::read_dir(&corpus).expect("the corpus directory").count();
    assert!(entries >= 6, "the corpus lost seeded entries");
    let bytes = fs::read(corpus.join(name)).expect("a corpus file");
    let _decoded = decode_mcp_rotation_blob(&bytes);
}
