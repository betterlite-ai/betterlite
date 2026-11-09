#![cfg(test)]

use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::cache_type::{CacheMode, LiteLlmCacheType};
use super::call_type::{CachingSupportedCallType, SUPPORTED_CALL_TYPES};
use super::control::DynamicCacheControl;
use super::embedding::{CachedEmbedding, EMBEDDING_CACHE_FORMAT_VERSION};
use super::ping::{CachePingResponse, HealthCheckCacheParams};
use super::pipeline::{
    RedisPipelineIncrement, RedisPipelineLpop, RedisPipelineRpush, RedisPipelineSet,
};
use super::record::{CacheTypeRecord, CacheTypesError, MAX_CACHE_TYPE_BYTES, decode_cache_types};
use super::semantic::SemanticCacheScope;

fn assert_wire<Value>(value: &Value, wire: &str)
where
    Value: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let decoded: Value = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_eq!(&decoded, value);
    assert_eq!(serde_json::to_string(value).expect("a wire form"), wire);
}

fn assert_scalar_wire<Value>(value: &Value, wire: &str)
where
    Value: Serialize + DeserializeOwned + PartialEq + Debug,
{
    assert_wire(value, &format!("\"{wire}\""));
}

#[rstest]
#[case::local(LiteLlmCacheType::Local, "local")]
#[case::redis(LiteLlmCacheType::Redis, "redis")]
#[case::redis_semantic(LiteLlmCacheType::RedisSemantic, "redis-semantic")]
#[case::valkey_semantic(LiteLlmCacheType::ValkeySemantic, "valkey-semantic")]
#[case::s3(LiteLlmCacheType::S3, "s3")]
#[case::disk(LiteLlmCacheType::Disk, "disk")]
#[case::qdrant_semantic(LiteLlmCacheType::QdrantSemantic, "qdrant-semantic")]
#[case::azure_blob(LiteLlmCacheType::AzureBlob, "azure-blob")]
#[case::gcs(LiteLlmCacheType::Gcs, "gcs")]
fn cache_type_round_trips(#[case] cache_type: LiteLlmCacheType, #[case] wire: &str) {
    assert_scalar_wire(&cache_type, wire);
}

#[rstest]
#[case::default_on(CacheMode::DefaultOn, "default_on")]
#[case::default_off(CacheMode::DefaultOff, "default_off")]
fn cache_mode_round_trips(#[case] mode: CacheMode, #[case] wire: &str) {
    assert_scalar_wire(&mode, wire);
}

#[rstest]
#[case::key(SemanticCacheScope::Key, "key")]
#[case::end_user(SemanticCacheScope::EndUser, "end_user")]
fn semantic_scope_round_trips(#[case] scope: SemanticCacheScope, #[case] wire: &str) {
    assert_scalar_wire(&scope, wire);
}

#[rstest]
#[case::completion(CachingSupportedCallType::Completion, "completion")]
#[case::acompletion(CachingSupportedCallType::Acompletion, "acompletion")]
#[case::embedding(CachingSupportedCallType::Embedding, "embedding")]
#[case::aembedding(CachingSupportedCallType::Aembedding, "aembedding")]
#[case::atranscription(CachingSupportedCallType::Atranscription, "atranscription")]
#[case::transcription(CachingSupportedCallType::Transcription, "transcription")]
#[case::atext_completion(CachingSupportedCallType::AtextCompletion, "atext_completion")]
#[case::text_completion(CachingSupportedCallType::TextCompletion, "text_completion")]
#[case::arerank(CachingSupportedCallType::Arerank, "arerank")]
#[case::rerank(CachingSupportedCallType::Rerank, "rerank")]
#[case::responses(CachingSupportedCallType::Responses, "responses")]
#[case::aresponses(CachingSupportedCallType::Aresponses, "aresponses")]
#[case::anthropic_messages(CachingSupportedCallType::AnthropicMessages, "anthropic_messages")]
#[case::aanthropic_messages(CachingSupportedCallType::AanthropicMessages, "aanthropic_messages")]
fn supported_call_type_round_trips(
    #[case] call_type: CachingSupportedCallType,
    #[case] wire: &str,
) {
    assert_scalar_wire(&call_type, wire);
}

#[test]
fn default_supported_call_types_match_the_declared_order() {
    let expected = [
        "completion",
        "acompletion",
        "embedding",
        "aembedding",
        "atranscription",
        "transcription",
        "atext_completion",
        "text_completion",
        "arerank",
        "rerank",
        "responses",
        "aresponses",
        "anthropic_messages",
        "aanthropic_messages",
    ];
    let wires = SUPPORTED_CALL_TYPES
        .iter()
        .map(|call_type| serde_json::to_string(call_type).expect("a wire form"))
        .collect::<Vec<String>>();
    let quoted = expected
        .iter()
        .map(|wire| format!("\"{wire}\""))
        .collect::<Vec<String>>();
    assert_eq!(wires, quoted);
}

#[rstest]
#[case::full(concat!(
    r#"{"status":"success","cache_type":"local","ping_response":true,"#,
    r#""set_cache_response":"ok","litellm_cache_params":"{\"a\":1}","#,
    r#""health_check_cache_params":{"host":"localhost","port":6379,"#,
    r#""redis_kwargs":{"db":0,"ssl":false},"namespace":"ns","#,
    r#""redis_version":"7.0.1"}}"#
))]
#[case::minimal(concat!(
    r#"{"status":"failed","cache_type":"redis","ping_response":null,"#,
    r#""set_cache_response":null,"litellm_cache_params":null,"#,
    r#""health_check_cache_params":null}"#
))]
fn cache_ping_response_round_trips(#[case] wire: &str) {
    let decoded: CachePingResponse = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[test]
fn cache_ping_response_rejects_an_unmodelled_health_check_key() {
    let input = concat!(
        r#"{"status":"success","cache_type":"local","ping_response":null,"#,
        r#""set_cache_response":null,"litellm_cache_params":null,"#,
        r#""health_check_cache_params":{"region":"eu"}}"#
    );
    let error =
        serde_json::from_str::<CachePingResponse>(input).expect_err("an unmodelled health key");
    assert!(error.to_string().contains("region"));
}

#[rstest]
#[case::minimal(
    r#"{"host":null,"port":null,"redis_kwargs":null,"namespace":null,"redis_version":null}"#
)]
#[case::text_port(
    r#"{"host":null,"port":"6379","redis_kwargs":null,"namespace":null,"redis_version":7}"#
)]
#[case::full(
    r#"{"host":"cache","port":6379,"redis_kwargs":{"db":1},"namespace":"team","redis_version":7.5}"#
)]
fn health_check_params_round_trip(#[case] wire: &str) {
    let decoded: HealthCheckCacheParams = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[test]
fn health_check_params_reject_a_fractional_port() {
    let input = concat!(
        r#"{"host":null,"port":6379.5,"redis_kwargs":null,"namespace":null,"#,
        r#""redis_version":null}"#
    );
    let error = serde_json::from_str::<HealthCheckCacheParams>(input)
        .expect_err("a fractional port is not a declared port type");
    assert!(error.to_string().contains("TextOrInteger"));
}

#[rstest]
#[case::vector(concat!(
    r#"{"embedding":[0.1,0.2],"index":0,"object":"embedding","#,
    r#""model":"text-embedding-3-small","prompt_tokens":4,"#,
    r#""prompt_tokens_details":{"image_tokens":2},"format_version":2}"#
))]
#[case::text(concat!(
    r#"{"embedding":"base64","index":null,"object":null,"model":null,"#,
    r#""prompt_tokens":null,"prompt_tokens_details":null,"format_version":2}"#
))]
fn cached_embedding_round_trips(#[case] wire: &str) {
    let decoded: CachedEmbedding = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[test]
fn embedding_cache_format_version_is_two() {
    assert_eq!(EMBEDDING_CACHE_FORMAT_VERSION, 2);
}

#[test]
fn dynamic_cache_control_round_trips() {
    let control = DynamicCacheControl {
        ttl: Some(120),
        namespace: Some("team".to_owned()),
        s_maxage: Some(60),
        s_max_age: Some(30),
        no_cache: Some(false),
        no_store: Some(true),
    };
    let wire = concat!(
        r#"{"ttl":120,"namespace":"team","s-maxage":60,"s-max-age":30,"#,
        r#""no-cache":false,"no-store":true}"#
    );
    assert_wire(&control, wire);
    let empty = DynamicCacheControl::default();
    assert_eq!(serde_json::to_string(&empty).expect("a wire form"), "{}");
    assert_eq!(
        serde_json::from_str::<DynamicCacheControl>("{}").expect("a wire form"),
        empty
    );
}

#[test]
fn dynamic_cache_control_rejects_an_unknown_key() {
    assert!(matches!(
        serde_json::from_str::<DynamicCacheControl>(r#"{"max-age":60}"#),
        Err(error) if error.is_data()
    ));
}

#[rstest]
#[case::with_ttl(r#"{"key":"k","increment_value":1.5,"ttl":30}"#)]
#[case::without_ttl(r#"{"key":"k","increment_value":2.0,"ttl":null}"#)]
fn redis_pipeline_increment_round_trips(#[case] wire: &str) {
    let decoded: RedisPipelineIncrement = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[rstest]
#[case::object(r#"{"key":"k","value":{"nested":[1,true,null]},"ttl":null}"#)]
#[case::text(r#"{"key":"k","value":"text","ttl":30}"#)]
fn redis_pipeline_set_round_trips(#[case] wire: &str) {
    let decoded: RedisPipelineSet = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[test]
fn redis_pipeline_rpush_round_trips() {
    let wire = r#"{"key":"k","values":["a",1,false]}"#;
    let decoded: RedisPipelineRpush = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[rstest]
#[case::count(r#"{"key":"k","count":5}"#)]
#[case::no_count(r#"{"key":"k","count":null}"#)]
fn redis_pipeline_lpop_round_trips(#[case] wire: &str) {
    let decoded: RedisPipelineLpop = serde_json::from_str(wire).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[rstest]
#[case::cache_type(r#"{"cache_type":"redis-semantic"}"#)]
#[case::cache_mode(r#"{"cache_mode":"default_off"}"#)]
#[case::semantic_scope(r#"{"semantic_scope":"end_user"}"#)]
#[case::call_type(r#"{"call_type":"anthropic_messages"}"#)]
#[case::dynamic_cache_control(r#"{"dynamic_cache_control":{"ttl":120}}"#)]
#[case::ping_response(
    r#"{"cache_ping_response":{"status":"success","cache_type":"local","ping_response":true,"set_cache_response":"ok","litellm_cache_params":null,"health_check_cache_params":null}}"#
)]
#[case::health_check_params(
    r#"{"health_check_cache_params":{"host":null,"port":6379,"redis_kwargs":null,"namespace":null,"redis_version":null}}"#
)]
#[case::cached_embedding(
    r#"{"cached_embedding":{"embedding":null,"index":null,"object":null,"model":null,"prompt_tokens":null,"prompt_tokens_details":null,"format_version":2}}"#
)]
#[case::pipeline_increment(
    r#"{"redis_pipeline_increment":{"key":"k","increment_value":1.5,"ttl":30}}"#
)]
#[case::pipeline_set(r#"{"redis_pipeline_set":{"key":"k","value":"v","ttl":null}}"#)]
#[case::pipeline_rpush(r#"{"redis_pipeline_rpush":{"key":"k","values":["a"]}}"#)]
#[case::pipeline_lpop(r#"{"redis_pipeline_lpop":{"key":"k","count":null}}"#)]
fn cache_type_records_round_trip(#[case] wire: &str) {
    let decoded = decode_cache_types(wire.as_bytes()).expect("a LiteLLM input form");
    assert_wire(&decoded, wire);
}

#[test]
fn cache_type_record_variants_are_distinct() {
    let decoded = decode_cache_types(br#"{"cache_type":"redis"}"#).expect("a record");
    assert!(matches!(
        decoded,
        CacheTypeRecord::CacheType(LiteLlmCacheType::Redis)
    ));
}

#[test]
fn decode_rejects_input_past_the_byte_limit() {
    let oversized = vec![b' '; MAX_CACHE_TYPE_BYTES + 1];
    assert!(matches!(
        decode_cache_types(&oversized),
        Err(CacheTypesError::TooLarge {
            bytes,
            limit: MAX_CACHE_TYPE_BYTES,
        }) if bytes == MAX_CACHE_TYPE_BYTES + 1
    ));
}

#[rstest]
fn replays_the_cache_types_corpus(#[files("fuzz/corpus/cache_types-*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus seed");
    let name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .expect("a seed name")
        .to_owned();
    if name.contains("invalid") {
        assert!(
            matches!(
                decode_cache_types(&bytes),
                Err(CacheTypesError::Decode { .. })
            ),
            "{name} must not decode"
        );
    } else {
        let decoded = decode_cache_types(&bytes).expect("a valid seed");
        let encoded = serde_json::to_vec(&decoded).expect("a wire form");
        assert_eq!(
            decode_cache_types(&encoded).expect("a re-encoded seed"),
            decoded
        );
    }
}
