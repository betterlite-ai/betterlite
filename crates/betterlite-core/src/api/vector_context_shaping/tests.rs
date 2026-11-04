#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::chat::ChatMessage;
use indexmap::IndexMap;
use rstest::rstest;

use super::{
    FailureMode, FailureModeResolution, configured_failure_mode, context_message, context_messages,
    extract_query_from_messages, messages_with_context, requested_vector_store_ids, scan_request,
};

fn messages(source: &str) -> Vec<ChatMessage> {
    serde_json::from_str(source).expect("valid messages fixture")
}

fn params(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("valid non_default_params fixture")
}

fn search_response(source: &str) -> VectorStoreSearchResponse {
    serde_json::from_str(source).expect("valid search response fixture")
}

#[rstest]
#[case::last_message_wins(
    r#"[{"role":"user","content":"first"},{"role":"assistant","content":"last"}]"#,
    Some("last")
)]
#[case::empty_messages("[]", None)]
#[case::last_message_without_content(r#"[{"role":"user"}]"#, None)]
#[case::null_content(r#"[{"role":"user","content":null}]"#, None)]
#[case::empty_list_content(r#"[{"role":"user","content":[]}]"#, None)]
#[case::empty_string_content(r#"[{"role":"user","content":""}]"#, Some(""))]
#[case::multimodal_first_text_item(
    r#"[{"role":"user","content":[
        {"type":"image_url","image_url":{"url":"u"}},
        {"type":"text","text":"a"},
        {"type":"text","text":"b"}
    ]}]"#,
    Some("a")
)]
#[case::multimodal_empty_first_text_item(
    r#"[{"role":"user","content":[
        {"type":"text","text":""},
        {"type":"text","text":"second"}
    ]}]"#,
    Some("")
)]
fn query_uses_last_message(#[case] source: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        extract_query_from_messages(&messages(source)),
        expected.map(str::to_owned)
    );
}

#[test]
fn query_never_scans_backward_for_a_user_role() {
    let source = r#"[
        {"role":"user","content":"find me"},
        {"role":"assistant","content":"ignored by role, used because last"}
    ]"#;
    assert_eq!(
        extract_query_from_messages(&messages(source)),
        Some("ignored by role, used because last".to_owned())
    );
}

fn one_message(source: &str) -> ChatMessage {
    let mut built = messages(source);
    built.pop().expect("one message fixture")
}

#[test]
fn context_text_and_position() {
    let response = search_response(
        r#"{"object":"vector_store.search_results.page","data":[
            {"content":[{"text":"a"},{"text":""},{"text":"b"}]}
        ]}"#,
    );
    let message = context_message(&response).expect("context message");
    assert_eq!(
        message,
        one_message(r#"[{"role":"user","content":"Context:\n\na\n\nb\n\n"}]"#)
    );
}

#[rstest]
#[case::empty_data(r#"{"data":[]}"#)]
#[case::null_data(r#"{"data":null}"#)]
#[case::missing_data("{}")]
#[case::missing_content(r#"{"data":[{}]}"#)]
#[case::null_content(r#"{"data":[{"content":null}]}"#)]
#[case::all_falsey_text(r#"{"data":[{"content":[{"text":""},{"text":null}]}]}"#)]
fn context_message_yields_none(#[case] source: &str) {
    assert_eq!(context_message(&search_response(source)), None);
}

#[test]
fn context_message_ignores_content_type_when_text_is_present() {
    let response = search_response(r#"{"data":[{"content":[{"type":"other","text":"a"}]}]}"#);
    let message = context_message(&response).expect("context message");
    assert_eq!(
        message,
        one_message(r#"[{"role":"user","content":"Context:\n\na\n\n"}]"#)
    );
}

#[test]
fn context_messages_from_two_stores_keep_store_order() {
    let first = search_response(r#"{"data":[{"content":[{"text":"first"}]}]}"#);
    let second = search_response(r#"{"data":[{"content":[{"text":"second"}]}]}"#);
    let built = context_messages(&[first, second]);
    assert_eq!(
        built,
        messages(
            r#"[
                {"role":"user","content":"Context:\n\nfirst\n\n"},
                {"role":"user","content":"Context:\n\nsecond\n\n"}
            ]"#
        )
    );
}

#[test]
fn messages_with_context_inserts_before_the_final_original_message() {
    let originals = messages(r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#);
    let context = messages(r#"[{"role":"user","content":"ctx"}]"#);
    let result = messages_with_context(&originals, &context);
    assert_eq!(
        result,
        messages(
            r#"[
                {"role":"user","content":"a"},
                {"role":"user","content":"ctx"},
                {"role":"user","content":"b"}
            ]"#
        )
    );
}

#[test]
fn messages_with_context_returns_context_alone_with_no_originals() {
    let context = messages(r#"[{"role":"user","content":"ctx"}]"#);
    assert_eq!(messages_with_context(&[], &context), context);
}

#[test]
fn messages_with_context_returns_an_equal_copy_with_no_context() {
    let originals = messages(r#"[{"role":"user","content":"a"}]"#);
    assert_eq!(messages_with_context(&originals, &[]), originals);
}

#[test]
fn scan_request_merge() {
    let non_default_params = params(
        r#"{
            "x": 2,
            "policies": ["p"],
            "guardrail_config": {"a": "b"},
            "include_guardrail_response": true,
            "proxy_server_request": {"body": {"model": "body", "x": 1, "guardrails": ["g"]}}
        }"#,
    );
    let scanned = scan_request("argument", &non_default_params);
    assert_eq!(
        scanned,
        params(
            r#"{
                "model": "body",
                "x": 2,
                "proxy_server_request": {"body": {"model": "body", "x": 1, "guardrails": ["g"]}}
            }"#
        )
    );
}

#[test]
fn scan_request_merge_non_default_model_wins_over_body_model() {
    let non_default_params =
        params(r#"{"model": "non_default", "proxy_server_request": {"body": {"model": "body"}}}"#);
    let scanned = scan_request("argument", &non_default_params);
    assert_eq!(
        scanned.get("model"),
        Some(&Json::Text("non_default".to_owned()))
    );
}

#[rstest]
#[case::missing_proxy_request("{}")]
#[case::non_map_proxy_request(r#"{"proxy_server_request": "not-a-map"}"#)]
#[case::missing_body(r#"{"proxy_server_request": {}}"#)]
#[case::non_map_body(r#"{"proxy_server_request": {"body": "not-a-map"}}"#)]
fn scan_request_falls_back_and_retains_guardrail_keys(#[case] source: &str) {
    let mut non_default_params = params(source);
    non_default_params.extend(params(
        r#"{
            "guardrails": ["g"],
            "policies": ["p"],
            "guardrail_config": {"a": "b"},
            "include_guardrail_response": true
        }"#,
    ));
    let scanned = scan_request("argument", &non_default_params);
    assert_eq!(
        scanned.get("model"),
        Some(&Json::Text("argument".to_owned()))
    );
    for key in [
        "guardrails",
        "policies",
        "guardrail_config",
        "include_guardrail_response",
    ] {
        assert!(
            scanned.contains_key(key),
            "expected fallback to retain {key}"
        );
    }
}

#[test]
fn requested_ids_for_diagnostics() {
    let non_default_params = params(r#"{"vector_store_ids": ["vs_a", 2, null, true]}"#);
    assert_eq!(
        requested_vector_store_ids(&non_default_params),
        vec!["vs_a", "2", "None", "True"]
    );
}

#[rstest]
#[case::string(r#"{"vector_store_ids": "vs_a"}"#)]
#[case::map(r#"{"vector_store_ids": {"id": "vs_a"}}"#)]
#[case::null(r#"{"vector_store_ids": null}"#)]
#[case::missing("{}")]
fn requested_ids_for_diagnostics_yields_no_ids(#[case] source: &str) {
    assert_eq!(
        requested_vector_store_ids(&params(source)),
        Vec::<String>::new()
    );
}

#[test]
fn failure_policy() {
    assert_eq!(
        configured_failure_mode(&Json::Text("annotate".to_owned())),
        FailureModeResolution {
            mode: FailureMode::Annotate,
            warning: None
        }
    );
    assert_eq!(
        configured_failure_mode(&Json::Text("error".to_owned())),
        FailureModeResolution {
            mode: FailureMode::Error,
            warning: None
        }
    );
}

#[rstest]
#[case::bogus_string(Json::Text("bogus".to_owned()), "'bogus'")]
#[case::null(Json::Null, "None")]
#[case::integer(Json::Number(serde_json::Number::from(1_i32)), "1")]
fn failure_policy_falls_back_and_warns_on_unsupported_values(
    #[case] configured: Json,
    #[case] repr: &str,
) {
    let resolution = configured_failure_mode(&configured);
    assert_eq!(resolution.mode, FailureMode::Annotate);
    assert_eq!(
        resolution.warning,
        Some(format!(
            "Unsupported vector_store_search_failure_mode={repr}, falling back to 'annotate'. Supported modes: annotate, error"
        ))
    );
}
