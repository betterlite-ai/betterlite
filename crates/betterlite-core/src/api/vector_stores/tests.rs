#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::api::vector_stores::answer::decorate_answer;
use crate::api::vector_stores::streaming::decorate_chunk;
use crate::api::vector_stores::{Decoration, SEARCH_FAILURES, SEARCH_RESULTS};

const RESULTS: &str = r#"[{"search_query":"q","data":[]}]"#;
const FAILURES: &str =
    r#"[{"vector_store_id":"vs_a","custom_llm_provider":"openai","error":"failed"}]"#;
const NO_LOGGING: &str = "No litellm_logging_obj in request_data";
const NO_FIELDS: &str = "No search results or search failures found";
const NO_STREAM_FIELDS: &str = "No search results or search failures found for streaming chunk";
const RESPONSE_ERROR: &str = "Error adding search results to response: ";
const STREAM_ERROR: &str = "Error adding search results to streaming chunk: ";
const CHAT_ANSWER: &str = r#"{
    "id": "chat1",
    "object": "chat.completion",
    "choices": [
        {"index": 0, "message": {"role": "assistant", "content": "a", "provider_specific_fields": {"keep": "x", "search_results": "old"}}},
        {"index": 1, "message": {"role": "assistant", "content": "b", "provider_specific_fields": {"keep": "x", "search_results": "old"}}}
    ]
}"#;
const STALE_ANSWER: &str = r#"{
    "id": "chat1",
    "object": "chat.completion",
    "choices": [
        {"index": 0, "message": {"role": "assistant", "content": "a", "provider_specific_fields": {"keep": "x", "search_results": "old", "vector_store_search_failures": "oldf"}}}
    ]
}"#;
const RESPONSES_ANSWER: &str =
    r#"{"id":"r1","created_at":3,"object":"response","output":[{"type":"message","id":"msg1"}]}"#;
const CHUNK: &str = r#"{
    "id": "chunk",
    "object": "chat.completion.chunk",
    "choices": [
        {"index": 0, "delta": {"content": "a", "provider_specific_fields": {"keep": "x"}}},
        {"index": 1, "delta": {"content": "b"}}
    ]
}"#;
const STALE_CHUNK: &str = r#"{
    "id": "chunk",
    "object": "chat.completion.chunk",
    "choices": [
        {"index": 0, "delta": {"content": "a", "provider_specific_fields": {"keep": "x", "search_results": "old", "vector_store_search_failures": "oldf"}}},
        {"index": 1, "delta": {"content": "b"}}
    ]
}"#;

fn request(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("valid request_data fixture")
}

fn record(source: &str) -> Json {
    serde_json::from_str(source).expect("valid record fixture")
}

fn details_with(results: &str, failures: &str) -> String {
    format!(
        r#"{{"litellm_logging_obj":{{"model_call_details":{{"search_results":{results},"vector_store_search_failures":{failures}}}}}}}"#
    )
}

fn chunk_request(results: &str, failures: &str) -> IndexMap<String, Json> {
    request(&format!(
        r#"{{"search_results":{results},"vector_store_search_failures":{failures}}}"#
    ))
}

fn member_fields(answer: &Json, member: &str, index: usize) -> Option<Json> {
    let Json::Object(record) = answer else {
        return None;
    };
    let Some(Json::List(choices)) = record.get("choices") else {
        return None;
    };
    let Json::Object(choice) = choices.get(index)? else {
        return None;
    };
    let Some(Json::Object(target)) = choice.get(member) else {
        return None;
    };
    target.get("provider_specific_fields").cloned()
}

fn top_field(answer: &Json, key: &str) -> Option<Json> {
    let Json::Object(record) = answer else {
        return None;
    };
    record.get(key).cloned()
}

fn decorated(source_results: &str, source_failures: &str) -> Json {
    record(&format!(
        r#"{{"keep":"x","search_results":{source_results},"vector_store_search_failures":{source_failures}}}"#
    ))
}

fn supplied(results: &str, failures: &str) -> String {
    format!(r#"{{"search_results":{results},"vector_store_search_failures":{failures}}}"#)
}

#[test]
fn chat_retrieval_fields() {
    let request_data = request(&details_with(RESULTS, FAILURES));
    let mut answer = record(CHAT_ANSWER);
    let decoration = decorate_answer(&request_data, &mut answer);
    assert_eq!(
        decoration,
        Decoration {
            returns_answer: true,
            diagnostics: Vec::new(),
        }
    );
    let expected = decorated(RESULTS, FAILURES);
    assert_eq!(member_fields(&answer, "message", 0), Some(expected.clone()));
    assert_eq!(member_fields(&answer, "message", 1), Some(expected));
    assert_eq!(top_field(&answer, SEARCH_RESULTS), None);
    assert_eq!(top_field(&answer, SEARCH_FAILURES), None);
    assert_eq!(top_field(&answer, "id"), Some(record("\"chat1\"")));
}

#[rstest]
#[case::results_only(
    RESULTS,
    "[]",
    r#"{"id":"chat1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"a","provider_specific_fields":{"keep":"x","search_results":[{"search_query":"q","data":[]}],"vector_store_search_failures":"oldf"}}}]}"#
)]
#[case::failures_only(
    "[]",
    FAILURES,
    r#"{"id":"chat1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"a","provider_specific_fields":{"keep":"x","search_results":"old","vector_store_search_failures":[{"vector_store_id":"vs_a","custom_llm_provider":"openai","error":"failed"}]}}}]}"#
)]
fn empty_inputs_do_not_erase_existing_fields(
    #[case] results: &str,
    #[case] failures: &str,
    #[case] expected: &str,
) {
    let mut answer = record(STALE_ANSWER);
    let decoration = decorate_answer(&request(&details_with(results, failures)), &mut answer);
    assert!(decoration.returns_answer);
    assert_eq!(answer, record(expected));
}

#[rstest]
#[case::absent(
    r#"{"id":"c","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"a"}}]}"#
)]
#[case::null_fields(
    r#"{"id":"c","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"a","provider_specific_fields":null}}]}"#
)]
#[case::empty_fields(
    r#"{"id":"c","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"a","provider_specific_fields":{}}}]}"#
)]
fn a_message_without_provider_fields_gains_them(#[case] source: &str) {
    let request_data = request(&details_with(RESULTS, FAILURES));
    let mut answer = record(source);
    let decoration = decorate_answer(&request_data, &mut answer);
    assert_eq!(
        decoration,
        Decoration {
            returns_answer: true,
            diagnostics: Vec::new(),
        }
    );
    let expected = supplied(RESULTS, FAILURES);
    assert_eq!(
        member_fields(&answer, "message", 0),
        Some(record(&expected))
    );
}

#[test]
fn responses_only_receive_failures() {
    let request_data = request(&details_with(RESULTS, FAILURES));
    let mut answer = record(RESPONSES_ANSWER);
    let decoration = decorate_answer(&request_data, &mut answer);
    assert_eq!(
        decoration,
        Decoration {
            returns_answer: true,
            diagnostics: Vec::new(),
        }
    );
    assert_eq!(top_field(&answer, SEARCH_FAILURES), Some(record(FAILURES)));
    assert_eq!(top_field(&answer, SEARCH_RESULTS), None);
    assert_eq!(top_field(&answer, "id"), Some(record("\"r1\"")));
    assert_eq!(
        top_field(&answer, "output"),
        Some(record(r#"[{"type":"message","id":"msg1"}]"#))
    );
}

#[test]
fn responses_with_only_results_stay_unchanged() {
    let request_data = request(&details_with(RESULTS, "[]"));
    let mut answer = record(RESPONSES_ANSWER);
    let decoration = decorate_answer(&request_data, &mut answer);
    assert_eq!(
        decoration,
        Decoration {
            returns_answer: true,
            diagnostics: Vec::new(),
        }
    );
    assert_eq!(answer, record(RESPONSES_ANSWER));
}

#[test]
fn a_responses_object_never_decorates_choices() {
    let source = r#"{"id":"r1","object":"response","choices":[{"index":0,"message":{"role":"assistant","content":"a"}}]}"#;
    let request_data = request(&details_with(RESULTS, FAILURES));
    let mut answer = record(source);
    let decoration = decorate_answer(&request_data, &mut answer);
    assert_eq!(
        decoration,
        Decoration {
            returns_answer: true,
            diagnostics: Vec::new(),
        }
    );
    assert_eq!(member_fields(&answer, "message", 0), None);
    assert_eq!(top_field(&answer, SEARCH_FAILURES), Some(record(FAILURES)));
}

#[rstest]
#[case::no_choices(r#"{"id":"c","object":"chat.completion"}"#)]
#[case::empty_choices(r#"{"id":"c","object":"chat.completion","choices":[]}"#)]
#[case::choices_not_a_list(r#"{"id":"c","object":"chat.completion","choices":"none"}"#)]
#[case::choice_without_message(r#"{"id":"c","object":"chat.completion","choices":[{"index":0}]}"#)]
#[case::null_message(
    r#"{"id":"c","object":"chat.completion","choices":[{"index":0,"message":null}]}"#
)]
#[case::scalar_choice(r#"{"id":"c","object":"chat.completion","choices":[3]}"#)]
#[case::scalar_answer("3")]
fn chat_choices_without_a_message_are_unchanged(#[case] source: &str) {
    let mut answer = record(source);
    assert!(
        decorate_answer(&request(&details_with(RESULTS, FAILURES)), &mut answer).returns_answer
    );
    assert_eq!(answer, record(source));
}

#[rstest]
#[case::no_logging_object("{}", NO_LOGGING)]
#[case::null_logging_object(r#"{"litellm_logging_obj":null}"#, NO_LOGGING)]
#[case::absent_details(r#"{"litellm_logging_obj":{}}"#, NO_FIELDS)]
#[case::null_details(r#"{"litellm_logging_obj":{"model_call_details":null}}"#, NO_FIELDS)]
#[case::empty_details(r#"{"litellm_logging_obj":{"model_call_details":{}}}"#, NO_FIELDS)]
#[case::empty_sequences(
    r#"{"litellm_logging_obj":{"model_call_details":{"search_results":[],"vector_store_search_failures":[]}}}"#,
    NO_FIELDS
)]
#[case::fields_only_in_request_data(
    r#"{"litellm_logging_obj":{},"search_results":[{"search_query":"q","data":[]}],"vector_store_search_failures":[]}"#,
    NO_FIELDS
)]
fn successful_hook_noop(#[case] source: &str, #[case] diagnostic: &str) {
    let mut answer = record(CHAT_ANSWER);
    let decoration = decorate_answer(&request(source), &mut answer);
    assert_eq!(
        decoration,
        Decoration {
            returns_answer: false,
            diagnostics: vec![diagnostic.to_owned()],
        }
    );
    assert_eq!(answer, record(CHAT_ANSWER));
}

#[rstest]
#[case::logging_not_an_object(
    r#"{"litellm_logging_obj":"log"}"#,
    "the litellm_logging_obj field must be an object"
)]
#[case::details_not_an_object(
    r#"{"litellm_logging_obj":{"model_call_details":"details"}}"#,
    "the litellm_logging_obj.model_call_details field must be an object"
)]
#[case::results_not_a_list(
    r#"{"litellm_logging_obj":{"model_call_details":{"search_results":"results"}}}"#,
    "the search_results field must be a list of retrieval records"
)]
#[case::failures_not_a_list(
    r#"{"litellm_logging_obj":{"model_call_details":{"vector_store_search_failures":7}}}"#,
    "the vector_store_search_failures field must be a list of retrieval records"
)]
fn unreadable_metadata_is_reported(#[case] source: &str, #[case] diagnostic: &str) {
    let mut answer = record(CHAT_ANSWER);
    let decoration = decorate_answer(&request(source), &mut answer);
    assert!(!decoration.returns_answer);
    assert_eq!(
        decoration.diagnostics,
        vec![format!("{RESPONSE_ERROR}{diagnostic}")]
    );
    assert_eq!(answer, record(CHAT_ANSWER));
}

#[rstest]
#[case::provider_fields_not_an_object(
    r#"[
        {"index":0,"message":{"role":"assistant","content":"a","provider_specific_fields":{"keep":"x"}}},
        {"index":1,"message":{"role":"assistant","content":"b","provider_specific_fields":"bad"}}
    ]"#,
    "choice 1 has provider_specific_fields that are not an object"
)]
#[case::message_not_an_object(
    r#"[{"index":0,"message":"bad"}]"#,
    "choice 0 has a message that is not an object"
)]
fn successful_hook_reports_a_malformed_target(#[case] choices: &str, #[case] diagnostic: &str) {
    let request_data = request(&details_with(RESULTS, FAILURES));
    let mut answer = record(&format!(
        r#"{{"id":"c","object":"chat.completion","choices":{choices}}}"#
    ));
    let decoration = decorate_answer(&request_data, &mut answer);
    assert_eq!(
        decoration,
        Decoration {
            returns_answer: false,
            diagnostics: vec![format!("{RESPONSE_ERROR}{diagnostic}")],
        }
    );
}

#[test]
fn a_malformed_target_keeps_the_choices_decorated_before_it() {
    let choices = r#"[
        {"index":0,"message":{"role":"assistant","content":"a","provider_specific_fields":{"keep":"x"}}},
        {"index":1,"message":{"role":"assistant","content":"b","provider_specific_fields":"bad"}}
    ]"#;
    let request_data = request(&details_with(RESULTS, FAILURES));
    let mut answer = record(&format!(
        r#"{{"id":"c","object":"chat.completion","choices":{choices}}}"#
    ));
    let decoration = decorate_answer(&request_data, &mut answer);
    assert!(!decoration.returns_answer);
    assert_eq!(
        member_fields(&answer, "message", 0),
        Some(decorated(RESULTS, FAILURES))
    );
}

#[rstest]
#[case::both(
    RESULTS,
    FAILURES,
    RESULTS,
    FAILURES,
    r#"{"search_results":[{"search_query":"q","data":[]}],"vector_store_search_failures":[{"vector_store_id":"vs_a","custom_llm_provider":"openai","error":"failed"}]}"#
)]
#[case::results_only(
    RESULTS,
    "[]",
    RESULTS,
    r#""oldf""#,
    r#"{"search_results":[{"search_query":"q","data":[]}]}"#
)]
#[case::failures_only(
    "[]",
    FAILURES,
    r#""old""#,
    FAILURES,
    r#"{"vector_store_search_failures":[{"vector_store_id":"vs_a","custom_llm_provider":"openai","error":"failed"}]}"#
)]
fn streaming_retrieval_fields(
    #[case] results: &str,
    #[case] failures: &str,
    #[case] expected_results: &str,
    #[case] expected_failures: &str,
    #[case] expected_new: &str,
) {
    let mut chunk = record(STALE_CHUNK);
    let diagnostics = decorate_chunk(&chunk_request(results, failures), &mut chunk);
    assert!(diagnostics.is_empty(), "the hook reported {diagnostics:?}");
    assert_eq!(
        member_fields(&chunk, "delta", 0),
        Some(decorated(expected_results, expected_failures))
    );
    assert_eq!(
        member_fields(&chunk, "delta", 1),
        Some(record(expected_new))
    );
    assert_eq!(top_field(&chunk, "id"), Some(record("\"chunk\"")));
}

#[test]
fn streaming_choices_without_a_delta_are_unchanged() {
    let source = r#"{"id":"c","object":"chat.completion.chunk","choices":[
        {"index":0,"finish_reason":"stop"},
        {"index":1,"delta":null},
        {"index":2,"delta":{"content":"a"}}]}"#;
    let mut chunk = record(source);
    let diagnostics = decorate_chunk(&chunk_request(RESULTS, FAILURES), &mut chunk);
    assert_eq!(diagnostics, Vec::<String>::new());
    let expected = record(&format!(
        r#"{{"id":"c","object":"chat.completion.chunk","choices":[
        {{"index":0,"finish_reason":"stop"}},
        {{"index":1,"delta":null}},
        {{"index":2,"delta":{{"content":"a","provider_specific_fields":{}}}}}]}}"#,
        supplied(RESULTS, FAILURES)
    ));
    assert_eq!(chunk, expected);
}

#[rstest]
#[case::empty_request_data("{}", CHUNK)]
#[case::null_results(r#"{"search_results":null}"#, CHUNK)]
#[case::empty_sequences(r#"{"search_results":[],"vector_store_search_failures":[]}"#, CHUNK)]
#[case::logging_object_only(
    r#"{"litellm_logging_obj":{"model_call_details":{"search_results":[{"search_query":"q","data":[]}],"vector_store_search_failures":[{"vector_store_id":"vs_a","custom_llm_provider":"openai","error":"failed"}]}}}"#,
    CHUNK
)]
#[case::no_choices("{}", r#"{"id":"c","object":"chat.completion.chunk"}"#)]
#[case::delta_without_provider_fields(
    "{}",
    r#"{"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"a"}}]}"#
)]
fn streaming_without_fields_returns_the_chunk(
    #[case] request_source: &str,
    #[case] chunk_source: &str,
) {
    let mut chunk = record(chunk_source);
    let diagnostics = decorate_chunk(&request(request_source), &mut chunk);
    assert_eq!(diagnostics, vec![NO_STREAM_FIELDS.to_owned()]);
    assert_eq!(chunk, record(chunk_source));
}

#[test]
fn streaming_reports_unreadable_metadata() {
    let mut chunk = record(CHUNK);
    let diagnostics = decorate_chunk(&request(r#"{"search_results":"results"}"#), &mut chunk);
    let expected = vec![format!(
        "{STREAM_ERROR}the search_results field must be a list of retrieval records"
    )];
    assert_eq!((diagnostics, chunk), (expected, record(CHUNK)));
}

#[test]
fn streaming_reports_a_malformed_target_without_rollback() {
    let request_data = chunk_request(RESULTS, FAILURES);
    let mut chunk = record(
        r#"{"id":"c","object":"chat.completion.chunk","choices":[
            {"index":0,"delta":{"content":"a","provider_specific_fields":{"keep":"x"}}},
            {"index":1,"delta":{"content":"b","provider_specific_fields":"bad"}}]}"#,
    );
    let diagnostics = decorate_chunk(&request_data, &mut chunk);
    assert_eq!(
        diagnostics,
        vec![format!(
            "{STREAM_ERROR}choice 1 has provider_specific_fields that are not an object"
        )]
    );
    assert_eq!(
        member_fields(&chunk, "delta", 0),
        Some(decorated(RESULTS, FAILURES))
    );
}
