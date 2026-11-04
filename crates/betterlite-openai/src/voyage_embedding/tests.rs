#![cfg(test)]

use serde_json::{Map, Value, json};

use super::{
    VoyageContextConfig, VoyageEmbeddingConfig, VoyageEmbeddingVariant,
    decode_voyage_embedding_reply, is_contextual_model, voyage_credentials, voyage_headers,
};

#[test]
fn contextual_empty_input_gets_chunking_defaults() {
    let request = super::request(
        "voyage-context-3",
        json!([]),
        &Map::new(),
        VoyageEmbeddingVariant::Contextual,
    );
    assert_eq!(request["inputs"], json!([]));
    assert_eq!(request["enable_auto_chunking"], true);
    assert_eq!(request["chunk_size"], 32000);
}

#[test]
fn contextual_query_and_options_override_defaults() {
    let options = serde_json::from_value(json!({"input_type":"query", "chunk_size":0})).unwrap();
    let request = super::request(
        "m",
        json!("question"),
        &options,
        VoyageEmbeddingVariant::Contextual,
    );
    assert_eq!(request["inputs"], json!(["question"]));
    assert_eq!(request["input_type"], "query");
    assert_eq!(request["chunk_size"], 0);
    assert!(request.get("enable_auto_chunking").is_none());
}

#[test]
fn text_embedding_maps_dimensions_and_preserves_nulls() {
    let config = VoyageEmbeddingConfig {
        api_base: Some("https://example.test/v1/".to_owned()),
    };
    let options = serde_json::from_value(json!({"dimensions":8, "encoding_format":null})).unwrap();
    let request = super::request("m", json!(["a"]), &options, VoyageEmbeddingVariant::Text);
    assert_eq!(
        super::endpoint(config.api_base.as_deref(), VoyageEmbeddingVariant::Text),
        "https://example.test/v1//embeddings"
    );
    assert_eq!(request["output_dimension"], 8);
    assert_eq!(request["encoding_format"], Value::Null);
}

#[test]
fn reply_projection_keeps_missing_values_null_and_total_usage() {
    let decoded = decode_voyage_embedding_reply(
        br#"{"usage":{"total_tokens":7}}"#,
        503,
        VoyageEmbeddingVariant::Text,
    )
    .unwrap();
    assert_eq!(decoded["model"], Value::Null);
    assert_eq!(decoded["data"], Value::Null);
    assert_eq!(decoded["object"], Value::Null);
    assert_eq!(decoded["usage"]["prompt_tokens"], 7);
    assert_eq!(decoded["usage"]["total_tokens"], 7);
}

#[test]
fn credentials_fall_through_in_alias_order() {
    let result = voyage_credentials(None, |name| match name {
        "VOYAGE_API_KEY" => Some(String::new()),
        "VOYAGE_AI_API_KEY" => Some("key".to_owned()),
        _ => Some("later".to_owned()),
    });
    assert_eq!(result, "Bearer key");
}

#[rstest::rstest]
fn replays_the_voyage_embedding_reply_corpus(
    #[files("fuzz/corpus/voyage_embedding_reply/*")] path: std::path::PathBuf,
) {
    let bytes = std::fs::read(path).expect("corpus file");
    let _unused = decode_voyage_embedding_reply(&bytes, 502, VoyageEmbeddingVariant::Text);
}

#[test]
fn headers_are_replaced_and_missing_credentials_keep_none_text() {
    let supplied = serde_json::from_value(json!({"X-Caller": "discard"})).unwrap();
    let headers = voyage_headers(supplied, None, |_| None);
    assert_eq!(headers.len(), 1);
    assert_eq!(headers["Authorization"], "Bearer None");
}

#[test]
fn explicit_empty_credential_is_not_resolved_again() {
    let value = voyage_credentials(Some(""), |_| Some("fallback".to_owned()));
    assert_eq!(value, "Bearer ");
}

#[rstest::rstest]
#[case::contains_context("Voyage-Contextual", true)]
#[case::embedded_context("voyage_context_3", true)]
#[case::no_context("voyage-3", false)]
fn detects_context_anywhere_after_lowercasing(#[case] model: &str, #[case] expected: bool) {
    assert_eq!(is_contextual_model(model), expected);
}
