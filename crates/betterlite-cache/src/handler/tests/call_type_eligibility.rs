use rstest::rstest;

use super::support::{chat_kwargs, handler_named, kwargs, object, text};
use crate::cache_key::value::KeyValue;
use crate::handler::HandlerError;
use crate::handler::call_types::{
    call_type_name, call_type_supported, should_store_result_in_cache,
};
use crate::handler::state::CachingHandler;
use crate::types::call_type::CachingSupportedCallType;

const SUPPORTED: [CachingSupportedCallType; 3] = [
    CachingSupportedCallType::Acompletion,
    CachingSupportedCallType::Aembedding,
    CachingSupportedCallType::Aresponses,
];

fn handler_for(name: &str) -> CachingHandler {
    handler_named(name, &[], chat_kwargs())
}

#[test]
fn listed_call_type_is_supported() {
    let handler = handler_for("acompletion");
    assert!(call_type_supported(
        &handler.original_function.name,
        Some(&SUPPORTED)
    ));
}

#[test]
fn unlisted_call_type_is_not_supported() {
    let handler = handler_for("completion_with_retries");
    assert!(!call_type_supported(
        &handler.original_function.name,
        Some(&SUPPORTED)
    ));
}

#[test]
fn no_supported_list_means_no_cache_support() {
    let handler = handler_for("acompletion");
    assert!(!call_type_supported(&handler.original_function.name, None));
}

#[test]
fn empty_supported_list_supports_nothing() {
    let handler = handler_for("acompletion");
    assert!(!call_type_supported(
        &handler.original_function.name,
        Some(&[])
    ));
}

#[test]
fn aresponses_is_covered_by_responses() {
    let handler = handler_for("aresponses");
    assert!(call_type_supported(
        &handler.original_function.name,
        Some(&[CachingSupportedCallType::Responses])
    ));
}

#[test]
fn aresponses_is_covered_by_aresponses() {
    let handler = handler_for("aresponses");
    assert!(call_type_supported(
        &handler.original_function.name,
        Some(&[CachingSupportedCallType::Aresponses])
    ));
}

#[test]
fn responses_is_not_covered_by_aresponses() {
    let handler = handler_for("responses");
    assert!(!call_type_supported(
        &handler.original_function.name,
        Some(&[CachingSupportedCallType::Aresponses])
    ));
}

#[test]
fn wire_names_match_the_cache_call_types() {
    let handler = handler_for("aanthropic_messages");
    assert!(call_type_supported(
        &handler.original_function.name,
        Some(&[CachingSupportedCallType::AanthropicMessages])
    ));
}

#[test]
fn store_requires_a_supported_call_type() {
    let handler = handler_for("acompletion");
    assert!(
        should_store_result_in_cache(
            &handler.original_function.name,
            Some(&SUPPORTED),
            &chat_kwargs()
        )
        .expect("a plain request stores")
    );
    let empty_cache = kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache", object(&[])),
    ]);
    assert!(
        should_store_result_in_cache(
            &handler.original_function.name,
            Some(&SUPPORTED),
            &empty_cache
        )
        .expect("an empty cache dict stores")
    );
}

#[test]
fn store_follows_the_no_store_control() {
    let handler = handler_for("acompletion");
    let no_store_false = kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache", object(&[("no-store", KeyValue::Bool(false))])),
    ]);
    assert!(
        should_store_result_in_cache(
            &handler.original_function.name,
            Some(&SUPPORTED),
            &no_store_false
        )
        .expect("an explicit false stores")
    );
    let no_store_true = kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache", object(&[("no-store", KeyValue::Bool(true))])),
    ]);
    assert!(
        !should_store_result_in_cache(
            &handler.original_function.name,
            Some(&SUPPORTED),
            &no_store_true
        )
        .expect("an explicit true skips the store")
    );
}

#[test]
fn unsupported_call_type_skips_before_reading_the_cache_control() {
    let handler = handler_for("completion_with_retries");
    let malformed = kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache", text("not-an-object")),
    ]);
    assert!(
        !should_store_result_in_cache(
            &handler.original_function.name,
            Some(&SUPPORTED),
            &malformed
        )
        .expect("the unsupported path never reads the cache dict")
    );
}

#[test]
fn supported_call_type_errors_on_a_bad_cache_dict() {
    let handler = handler_for("acompletion");
    let malformed = kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache", text("not-an-object")),
    ]);
    let result = should_store_result_in_cache(
        &handler.original_function.name,
        Some(&SUPPORTED),
        &malformed,
    );
    assert!(matches!(
        result,
        Err(HandlerError::FieldType { field: "cache", .. })
    ));
}

#[test]
fn no_store_must_be_a_boolean() {
    let handler = handler_for("acompletion");
    let malformed = kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache", object(&[("no-store", text("yes"))])),
    ]);
    let result = should_store_result_in_cache(
        &handler.original_function.name,
        Some(&SUPPORTED),
        &malformed,
    );
    assert!(matches!(
        result,
        Err(HandlerError::FieldType {
            field: "cache.no-store",
            ..
        })
    ));
}

#[test]
fn a_null_cache_dict_takes_the_default() {
    let handler = handler_for("acompletion");
    let null_cache = kwargs(&[
        ("model", text("gpt-4")),
        ("messages", KeyValue::Array(Vec::new())),
        ("cache", KeyValue::Null),
    ]);
    assert!(
        should_store_result_in_cache(
            &handler.original_function.name,
            Some(&SUPPORTED),
            &null_cache
        )
        .expect("a null cache dict stores")
    );
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
fn call_type_names_match_the_wire_names(
    #[case] variant: CachingSupportedCallType,
    #[case] expected: &str,
) {
    assert_eq!(call_type_name(variant), expected);
    assert!(call_type_supported(
        call_type_name(variant),
        Some(&[variant])
    ));
}
