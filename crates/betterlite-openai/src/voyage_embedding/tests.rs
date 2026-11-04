#![cfg(test)]

use std::{error::Error as _, fs, path::PathBuf};

use serde_json::{Map, Value, json};

use super::{
    VoyageEmbeddingConfig, VoyageEmbeddingVariant, decode_voyage_embedding_reply,
    voyage_credentials, voyage_headers,
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
    assert_eq!(request["chunk_size"], 32_000_u32);
}

#[test]
fn contextual_query_and_options_override_defaults() {
    let options = serde_json::from_str(r#"{"input_type":"query", "chunk_size":0}"#).unwrap();
    let request = super::request(
        "m",
        json!("question"),
        &options,
        VoyageEmbeddingVariant::Contextual,
    );
    assert_eq!(request["inputs"], json!(["question"]));
    assert_eq!(request["input_type"], "query");
    assert_eq!(request["chunk_size"], 0_u32);
    assert!(request.get("enable_auto_chunking").is_none());
}

#[test]
fn text_embedding_maps_dimensions_and_preserves_nulls() {
    let config = VoyageEmbeddingConfig {
        api_base: Some("https://example.test/v1/".to_owned()),
    };
    let options = serde_json::from_str(r#"{"dimensions":8, "encoding_format":null}"#).unwrap();
    let request = super::request("m", json!(["a"]), &options, VoyageEmbeddingVariant::Text);
    assert_eq!(
        super::endpoint(config.api_base.as_deref(), VoyageEmbeddingVariant::Text),
        "https://example.test/v1//embeddings"
    );
    assert_eq!(request["output_dimension"], 8_u32);
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
    assert_eq!(decoded["usage"]["prompt_tokens"], 7_u32);
    assert_eq!(decoded["usage"]["total_tokens"], 7_u32);
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
#[case::text(VoyageEmbeddingVariant::Text)]
#[case::contextual(VoyageEmbeddingVariant::Contextual)]
fn replays_the_voyage_embedding_reply_corpus(
    #[case] variant: VoyageEmbeddingVariant,
    #[files("fuzz/corpus/voyage_embedding_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _unused = decode_voyage_embedding_reply(&bytes, 502, variant);
}

#[test]
fn headers_are_replaced_and_missing_credentials_keep_none_text() {
    let supplied = serde_json::from_str(r#"{"X-Caller": "discard"}"#).unwrap();
    let headers = voyage_headers(supplied, voyage_credentials(None, |_| None));
    assert_eq!(headers.len(), 1);
    assert_eq!(headers["Authorization"], "Bearer None");
}

#[test]
fn explicit_empty_credential_is_not_resolved_again() {
    let value = voyage_credentials(Some(""), |_| Some("fallback".to_owned()));
    assert_eq!(value, "Bearer ");
}

#[rstest::rstest]
#[case::contains_context("Voyage-Contextual", VoyageEmbeddingVariant::Contextual)]
#[case::embedded_context("voyage_context_3", VoyageEmbeddingVariant::Contextual)]
#[case::no_context("voyage-3", VoyageEmbeddingVariant::Text)]
fn detects_context_anywhere_after_lowercasing(
    #[case] model: &str,
    #[case] expected: VoyageEmbeddingVariant,
) {
    assert_eq!(VoyageEmbeddingVariant::for_model(model), expected);
}

#[rstest::rstest]
#[case::text(VoyageEmbeddingVariant::Text, "https://api.voyageai.com/v1/embeddings")]
#[case::contextual(
    VoyageEmbeddingVariant::Contextual,
    "https://api.voyageai.com/v1/contextualizedembeddings"
)]
fn malformed_reply_keeps_status_and_variant_url(
    #[case] variant: VoyageEmbeddingVariant,
    #[case] expected_url: &str,
) {
    let error = decode_voyage_embedding_reply(b"not-json", 503, variant).unwrap_err();
    match error {
        super::VoyageEmbeddingError::Decode {
            status,
            url,
            message,
            ..
        } => {
            assert_eq!(status, 503);
            assert_eq!(url, expected_url);
            assert_eq!(message, "not-json");
        }
    }
}

#[test]
fn reply_projection_defaults_missing_usage_to_zero() {
    let decoded = decode_voyage_embedding_reply(b"{}", 200, VoyageEmbeddingVariant::Text)
        .expect("empty valid response projects");
    assert_eq!(decoded["usage"]["prompt_tokens"], 0_u32);
    assert_eq!(decoded["usage"]["total_tokens"], 0_u32);
}

#[rstest::rstest]
#[case::text(VoyageEmbeddingVariant::Text)]
#[case::contextual(VoyageEmbeddingVariant::Contextual)]
fn null_token_counts_default_to_zero(#[case] variant: VoyageEmbeddingVariant) {
    let decoded =
        decode_voyage_embedding_reply(br#"{"usage":{"total_tokens":null}}"#, 200, variant).unwrap();
    assert_eq!(
        decoded["usage"],
        json!({"prompt_tokens": 0, "total_tokens": 0})
    );
}

#[rstest::rstest]
#[case::text(VoyageEmbeddingVariant::Text)]
#[case::contextual(VoyageEmbeddingVariant::Contextual)]
fn dimensions_mapping_wins_over_provider_option(#[case] variant: VoyageEmbeddingVariant) {
    let options = serde_json::from_str(r#"{"dimensions":8,"output_dimension":16}"#).unwrap();
    let result = super::request("m", json!(["a"]), &options, variant);
    assert_eq!(result["output_dimension"], 8_u32);
    assert!(result.get("dimensions").is_none());
}

#[rstest::rstest]
#[case::text(
    VoyageEmbeddingVariant::Text,
    r#""text""#,
    "{}",
    r#"{"model":"m","input":"text"}"#
)]
#[case::flat(VoyageEmbeddingVariant::Contextual, r#"["a","b"]"#, "{}", r#"{"model":"m","inputs":["a","b"],"input_type":"document","enable_auto_chunking":true,"chunk_size":32000}"#)]
#[case::scalar(VoyageEmbeddingVariant::Contextual, r#""a""#, "{}", r#"{"model":"m","inputs":["a"],"input_type":"document","enable_auto_chunking":true,"chunk_size":32000}"#)]
#[case::nested(
    VoyageEmbeddingVariant::Contextual,
    r#"[["a","b"]]"#,
    "{}",
    r#"{"model":"m","inputs":[["a","b"]]}"#
)]
#[case::tokens(
    VoyageEmbeddingVariant::Contextual,
    "[1,2]",
    "{}",
    r#"{"model":"m","inputs":[1,2]}"#
)]
#[case::overrides(VoyageEmbeddingVariant::Contextual, r#"["a"]"#, r#"{"model":"other","inputs":[["replacement"]],"enable_auto_chunking":false,"chunk_size":0,"input_type":"other"}"#, r#"{"model":"other","inputs":[["replacement"]],"enable_auto_chunking":false,"chunk_size":0,"input_type":"other"}"#)]
fn request_matches_provider_body(
    #[case] variant: VoyageEmbeddingVariant,
    #[case] input: &str,
    #[case] options: &str,
    #[case] expected: &str,
) {
    let result = super::request(
        "m",
        serde_json::from_str(input).unwrap(),
        &serde_json::from_str(options).unwrap(),
        variant,
    );
    assert_eq!(result, serde_json::from_str::<Value>(expected).unwrap());
}

#[rstest::rstest]
#[case::text_default(
    None,
    VoyageEmbeddingVariant::Text,
    "https://api.voyageai.com/v1/embeddings"
)]
#[case::context_default(
    None,
    VoyageEmbeddingVariant::Contextual,
    "https://api.voyageai.com/v1/contextualizedembeddings"
)]
#[case::empty(
    Some(""),
    VoyageEmbeddingVariant::Text,
    "https://api.voyageai.com/v1/embeddings"
)]
#[case::text_suffix(
    Some("https://example.test/embeddings"),
    VoyageEmbeddingVariant::Text,
    "https://example.test/embeddings"
)]
#[case::context_suffix(
    Some("https://example.test/contextualizedembeddings"),
    VoyageEmbeddingVariant::Contextual,
    "https://example.test/contextualizedembeddings"
)]
#[case::context_slash(
    Some("https://example.test/"),
    VoyageEmbeddingVariant::Contextual,
    "https://example.test//contextualizedembeddings"
)]
#[case::append(
    Some("https://example.test"),
    VoyageEmbeddingVariant::Text,
    "https://example.test/embeddings"
)]
fn endpoints_preserve_exact_suffixes(
    #[case] base: Option<&str>,
    #[case] variant: VoyageEmbeddingVariant,
    #[case] expected: &str,
) {
    assert_eq!(super::endpoint(base, variant), expected);
}

#[test]
fn parsed_projection_preserves_reply_values() {
    let parsed: Value = serde_json::from_str(r#"{"model":"returned","object":"list","data":[{"embedding":[1,2],"index":0}],"usage":{"total_tokens":4,"prompt_tokens":99}}"#).unwrap();
    let result = super::project_response(&parsed);
    assert_eq!(result["model"], parsed["model"]);
    assert_eq!(result["object"], parsed["object"]);
    assert_eq!(result["data"], parsed["data"]);
    assert_eq!(result["usage"]["prompt_tokens"], 4_u32);
    assert_eq!(result["usage"]["total_tokens"], 4_u32);
}

#[test]
fn decode_failure_retains_its_json_source() {
    let error =
        decode_voyage_embedding_reply(&[255_u8], 502, VoyageEmbeddingVariant::Text).unwrap_err();
    assert!(error.source().is_some());
}

#[rstest::rstest]
#[case::primary(None, Some("first"), Some("second"), Some("third"), "Bearer first")]
#[case::secondary(None, None, Some("second"), Some("third"), "Bearer second")]
#[case::third(None, Some(""), Some(""), Some("third"), "Bearer third")]
#[case::empty(None, None, None, Some(""), "Bearer None")]
#[case::explicit(Some("supplied"), Some("first"), None, None, "Bearer supplied")]
fn credential_precedence_preserves_explicit_keys(
    #[case] supplied: Option<&str>,
    #[case] primary: Option<&str>,
    #[case] secondary: Option<&str>,
    #[case] third: Option<&str>,
    #[case] expected: &str,
) {
    let result = voyage_credentials(supplied, |name| match name {
        "VOYAGE_API_KEY" => primary.map(str::to_owned),
        "VOYAGE_AI_API_KEY" => secondary.map(str::to_owned),
        "VOYAGE_AI_TOKEN" => third.map(str::to_owned),
        _ => None,
    });
    assert_eq!(result, expected);
}
