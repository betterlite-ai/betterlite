#![cfg(test)]

use serde_json::{Map, Value, json};

use super::{VoyageMultimodalError, credentials, is_multimodal_embeddings, request};
use crate::voyage_embedding::{VoyageEmbeddingVariant, endpoint};

fn options(source: &str) -> Map<String, Value> {
    serde_json::from_str(source).expect("options")
}

#[rstest::rstest]
#[case::lowercase("voyage-multimodal-3")]
#[case::uppercase("Voyage-Multimodal-3")]
#[case::embedded("voyage_multimodal_3")]
fn selects_multimodal_by_substring(#[case] model: &str) {
    assert!(is_multimodal_embeddings(model));
}

#[rstest::rstest]
#[case::text("voyage-3")]
#[case::contextual("voyage-context-3")]
fn leaves_other_models_alone(#[case] model: &str) {
    assert!(!is_multimodal_embeddings(model));
}

#[test]
fn default_url_is_the_multimodal_endpoint() {
    assert_eq!(
        endpoint(None, VoyageEmbeddingVariant::Multimodal),
        "https://api.voyageai.com/v1/multimodalembeddings"
    );
}

#[test]
fn empty_base_falls_back_like_litellm_truthiness() {
    assert_eq!(
        endpoint(Some(""), VoyageEmbeddingVariant::Multimodal),
        "https://api.voyageai.com/v1/multimodalembeddings"
    );
}

#[rstest::rstest]
#[case::append(
    "https://example.test/v1",
    "https://example.test/v1/multimodalembeddings"
)]
#[case::suffix(
    "https://example.test/v1/multimodalembeddings",
    "https://example.test/v1/multimodalembeddings"
)]
#[case::slash(
    "https://example.test/v1/",
    "https://example.test/v1//multimodalembeddings"
)]
fn supplied_base_is_not_trimmed(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(
        endpoint(Some(base), VoyageEmbeddingVariant::Multimodal),
        expected
    );
}

#[rstest::rstest]
#[case::primary(Some("first"), None, None, Some("first"))]
#[case::second(None, Some("second"), None, Some("second"))]
#[case::token(None, None, Some("third"), Some("third"))]
#[case::empty_third(None, None, Some(""), None)]
#[case::explicit(Some("supplied"), Some("first"), Some("second"), Some("supplied"))]
#[case::explicit_empty(Some(""), None, None, None)]
fn credential_resolution_keeps_truthy_aliases(
    #[case] explicit: Option<&str>,
    #[case] primary: Option<&str>,
    #[case] secondary: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let resolved = credentials(explicit, |name| match name {
        "VOYAGE_API_KEY" => primary.map(str::to_owned),
        "VOYAGE_AI_API_KEY" => secondary.map(str::to_owned),
        "VOYAGE_AI_TOKEN" => Some(String::new()),
        _ => None,
    });
    match expected {
        Some(key) => assert_eq!(resolved.expect("key"), key),
        None => assert!(matches!(
            resolved,
            Err(VoyageMultimodalError::MissingApiKey)
        )),
    }
}

#[test]
fn wrapper_rejects_a_missing_key() {
    let error = credentials(None, |_| None).expect_err("no key");
    assert!(matches!(error, VoyageMultimodalError::MissingApiKey));
}

#[test]
fn token_alias_is_accepted_for_multimodal() {
    let key = credentials(None, |name| {
        (name == "VOYAGE_AI_TOKEN").then(|| "third".to_owned())
    })
    .expect("token alias");
    assert_eq!(key, "third");
}

#[test]
fn scalar_string_becomes_one_text_block() {
    let result = request("m", json!("hello"), &Map::new()).expect("request");
    assert_eq!(
        result,
        json!({"model": "m", "inputs": [{"content": [{"type": "text", "text": "hello"}]}]})
    );
}

#[test]
fn list_items_are_normalized_in_order() {
    let result = request("m", json!(["a", 7]), &Map::new()).expect("request");
    assert_eq!(
        result,
        json!({"model": "m", "inputs": [{"content": [{"type": "text", "text": "a"}]}, 7]})
    );
}

#[test]
fn content_objects_preserve_their_other_fields() {
    let input = json!([{"content": [{"type": "text", "text": "a"}], "id": "doc-1"}]);
    let result = request("m", input, &Map::new()).expect("request");
    assert_eq!(
        result["inputs"][0],
        json!({"content": [{"type": "text", "text": "a"}], "id": "doc-1"})
    );
}

#[rstest::rstest]
#[case::null(Value::Null)]
#[case::empty_list(json!([]))]
#[case::empty_string(json!(""))]
#[case::zero(json!(0))]
#[case::empty_object(json!({}))]
#[case::no(json!(false))]
fn falsey_content_becomes_an_empty_list(#[case] content: Value) {
    let input = json!([{"content": content}]);
    let result = request("m", input, &Map::new()).expect("request");
    assert_eq!(result["inputs"][0]["content"], json!([]));
}

#[test]
fn objects_without_content_pass_through() {
    let input = json!([{"foo": "bar"}]);
    let result = request("m", input, &Map::new()).expect("request");
    assert_eq!(result["inputs"][0], json!({"foo": "bar"}));
}

#[rstest::rstest]
#[case::text(json!({"type": "text", "text": "hi"}), json!({"type": "text", "text": "hi"}))]
#[case::url(
    json!({"type": "image_url", "image_url": "https://example.test/a.png"}),
    json!({"type": "image_url", "image_url": "https://example.test/a.png"})
)]
#[case::nested_url(
    json!({"type": "image_url", "image_url": {"url": "https://example.test/a.png"}}),
    json!({"type": "image_url", "image_url": "https://example.test/a.png"})
)]
#[case::empty_nested_url(
    json!({"type": "image_url", "image_url": {"url": ""}}),
    json!({"type": "image_url", "image_url": ""})
)]
#[case::data_prefix(
    json!({"type": "image_url", "image_url": "data:image/png;base64,AAAA"}),
    json!({"type": "image_base64", "image_base64": "AAAA"})
)]
#[case::data_prefix_without_comma(
    json!({"type": "image_url", "image_url": "data:image/png;base64"}),
    json!({"type": "image_base64", "image_base64": ""})
)]
#[case::numeric_url(
    json!({"type": "image_url", "image_url": 4}),
    json!({"type": "image_url", "image_url": 4})
)]
fn image_blocks_normalize_by_value(#[case] item: Value, #[case] expected: Value) {
    let input = json!([{"content": [item]}]);
    let result = request("m", input, &Map::new()).expect("request");
    assert_eq!(result["inputs"][0]["content"][0], expected);
}

#[test]
fn image_url_fields_outside_the_two_are_dropped() {
    let input = json!([{"content": [{
        "type": "image_url",
        "image_url": "https://example.test/a.png",
        "detail": "high"
    }]}]);
    let result = request("m", input, &Map::new()).expect("request");
    assert_eq!(
        result["inputs"][0]["content"][0],
        json!({"type": "image_url", "image_url": "https://example.test/a.png"})
    );
}

#[rstest::rstest]
#[case::missing(json!({"type": "image_url"}))]
#[case::null(json!({"type": "image_url", "image_url": null}))]
#[case::object_without_url(json!({"type": "image_url", "image_url": {}}))]
#[case::object_with_null_url(json!({"type": "image_url", "image_url": {"url": null}}))]
fn missing_image_url_is_rejected(#[case] item: Value) {
    let input = json!([{"content": [item]}]);
    let error = request("m", input, &Map::new()).expect_err("missing url");
    assert!(matches!(error, VoyageMultimodalError::MissingImageUrl));
}

#[rstest::rstest]
#[case::string(json!("text"))]
#[case::number(json!(3))]
fn non_list_content_is_rejected(#[case] content: Value) {
    let input = json!([{"content": content}]);
    let error = request("m", input, &Map::new()).expect_err("content type");
    assert!(matches!(error, VoyageMultimodalError::ContentNotList));
}

#[test]
fn non_object_content_items_are_rejected() {
    let input = json!([{"content": ["text"]}]);
    let error = request("m", input, &Map::new()).expect_err("item type");
    assert!(matches!(error, VoyageMultimodalError::ContentItemNotObject));
}

#[test]
fn dimensions_become_output_dimension() {
    let result = request("m", json!("a"), &options(r#"{"dimensions": 8}"#)).expect("request");
    assert_eq!(result["output_dimension"], 8_u32);
    assert!(result.get("dimensions").is_none());
}

#[test]
fn caller_options_overlay_model_and_inputs() {
    let result = request(
        "m",
        json!("a"),
        &options(r#"{"model": "other", "inputs": ["replacement"]}"#),
    )
    .expect("request");
    assert_eq!(result["model"], "other");
    assert_eq!(result["inputs"], json!(["replacement"]));
}

#[test]
fn mapped_dimensions_win_over_a_supplied_output_dimension() {
    let result = request(
        "m",
        json!("a"),
        &options(r#"{"dimensions": 8, "output_dimension": 16}"#),
    )
    .expect("request");
    assert_eq!(result["output_dimension"], 8_u32);
}
