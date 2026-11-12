use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::support::{cache_object, cache_text, cached_embedding, cached_embedding_fields};
use crate::handler::embedding::current_format_embedding_entry;
use crate::types::embedding::{CachedEmbedding, EmbeddingPayload};

fn entry_with(field: &str, value: CacheValue) -> CacheValue {
    let mut fields = cached_embedding_fields(2);
    fields.insert(field.to_owned(), value);
    CacheValue::Object(fields)
}

fn entry_without(field: &str) -> CacheValue {
    let mut fields: BTreeMap<String, CacheValue> = cached_embedding_fields(2);
    fields.remove(field);
    CacheValue::Object(fields)
}

fn number(value: f64) -> Number {
    Number::from_f64(value).expect("the fixture number is finite")
}

#[test]
fn accepts_a_current_format_entry() {
    let record = current_format_embedding_entry(&cached_embedding(2)).expect("the entry validates");
    assert_eq!(
        record,
        CachedEmbedding {
            embedding: Some(EmbeddingPayload::Vector(vec![number(0.5), number(-1.25)])),
            index: Some(0),
            object: Some("embedding".to_owned()),
            model: Some("text-embedding-3-small".to_owned()),
            prompt_tokens: Some(3),
            prompt_tokens_details: None,
            format_version: 2,
        }
    );
}

#[test]
fn accepts_null_optional_fields() {
    let entry = cache_object(&[
        ("embedding", CacheValue::Null),
        ("index", CacheValue::Null),
        ("object", CacheValue::Null),
        ("model", CacheValue::Null),
        ("prompt_tokens", CacheValue::Null),
        ("prompt_tokens_details", CacheValue::Null),
        ("format_version", CacheValue::Integer(2)),
    ]);
    let record = current_format_embedding_entry(&entry).expect("null fields validate");
    assert_eq!(record.embedding, None);
    assert_eq!(record.index, None);
    assert_eq!(record.object, None);
    assert_eq!(record.model, None);
    assert_eq!(record.prompt_tokens, None);
    assert_eq!(record.prompt_tokens_details, None);
}

#[test]
fn accepts_a_text_embedding() {
    let record =
        current_format_embedding_entry(&entry_with("embedding", cache_text("a-text-embedding")))
            .expect("text embeddings validate");
    assert_eq!(
        record.embedding,
        Some(EmbeddingPayload::Text("a-text-embedding".to_owned()))
    );
}

#[test]
fn accepts_an_empty_vector() {
    let record =
        current_format_embedding_entry(&entry_with("embedding", CacheValue::Array(Vec::new())))
            .expect("an empty vector validates");
    assert_eq!(record.embedding, Some(EmbeddingPayload::Vector(Vec::new())));
}

#[test]
fn accepts_integer_vector_components() {
    let record = current_format_embedding_entry(&entry_with(
        "embedding",
        CacheValue::Array(vec![CacheValue::Integer(2)]),
    ))
    .expect("integer components validate");
    assert_eq!(
        record.embedding,
        Some(EmbeddingPayload::Vector(vec![Number::from(2_i32)]))
    );
}

#[test]
fn accepts_a_prompt_tokens_details_object() {
    let record = current_format_embedding_entry(&entry_with(
        "prompt_tokens_details",
        cache_object(&[("image_tokens", CacheValue::Integer(4))]),
    ))
    .expect("token details validate");
    let details = record
        .prompt_tokens_details
        .expect("the token details survive");
    assert_eq!(
        details.get("image_tokens"),
        Some(&Json::Number(Number::from(4_i32)))
    );
}

#[test]
fn ignores_unknown_fields() {
    let record = current_format_embedding_entry(&entry_with(
        "a-field-from-the-future",
        CacheValue::Integer(1),
    ))
    .expect("unknown fields are ignored");
    assert_eq!(record.format_version, 2);
}

#[rstest]
#[case::version_one(1)]
#[case::version_zero(0)]
#[case::version_three(3)]
fn rejects_an_old_format_version(#[case] version: i64) {
    assert!(current_format_embedding_entry(&cached_embedding(version)).is_none());
}

#[rstest]
#[case::null(CacheValue::Null)]
#[case::text(cache_text("not-a-record"))]
#[case::array(CacheValue::Array(vec![CacheValue::Integer(2)]))]
#[case::integer(CacheValue::Integer(2))]
#[case::boolean(CacheValue::Boolean(true))]
fn rejects_non_object_entries(#[case] entry: CacheValue) {
    assert!(current_format_embedding_entry(&entry).is_none());
}

#[rstest]
#[case::embedding("embedding")]
#[case::index("index")]
#[case::object("object")]
#[case::model("model")]
#[case::prompt_tokens("prompt_tokens")]
#[case::prompt_tokens_details("prompt_tokens_details")]
#[case::format_version("format_version")]
fn rejects_missing_fields(#[case] field: &str) {
    assert!(current_format_embedding_entry(&entry_without(field)).is_none());
}

#[rstest]
#[case::embedding_boolean("embedding", CacheValue::Boolean(true))]
#[case::embedding_number("embedding", CacheValue::Integer(1))]
#[case::embedding_object("embedding", cache_object(&[]))]
#[case::embedding_vector_with_text(
    "embedding",
    CacheValue::Array(vec![cache_text("not-a-number")])
)]
#[case::embedding_vector_with_null(
    "embedding",
    CacheValue::Array(vec![CacheValue::Null])
)]
#[case::index_text("index", cache_text("0"))]
#[case::index_float("index", CacheValue::Float(1.0))]
#[case::object_integer("object", CacheValue::Integer(1))]
#[case::model_boolean("model", CacheValue::Boolean(false))]
#[case::prompt_tokens_text("prompt_tokens", cache_text("3"))]
#[case::prompt_tokens_float("prompt_tokens", CacheValue::Float(3.0))]
#[case::details_list("prompt_tokens_details", CacheValue::Array(Vec::new()))]
#[case::details_text("prompt_tokens_details", cache_text("details"))]
#[case::format_version_text("format_version", cache_text("2"))]
#[case::format_version_float("format_version", CacheValue::Float(2.0))]
#[case::format_version_null("format_version", CacheValue::Null)]
fn rejects_wrong_field_types(#[case] field: &str, #[case] value: CacheValue) {
    assert!(current_format_embedding_entry(&entry_with(field, value)).is_none());
}

#[test]
fn rejects_a_vector_with_a_non_finite_float() {
    let entry = entry_with(
        "embedding",
        CacheValue::Array(vec![CacheValue::Float(f64::NAN)]),
    );
    assert!(current_format_embedding_entry(&entry).is_none());
}
