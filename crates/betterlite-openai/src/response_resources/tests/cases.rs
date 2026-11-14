use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde_json::Value;

use super::{compact_call, header_map};
use crate::api::responses_shapes::DeleteResponseResult;
use crate::error::OpenAiError;
use crate::response_events::{RawResponse, ReplyBody, ResponsesReply};
use crate::response_input::ResponsesInput;
use crate::response_resources::error::ResponseResourceError;
use crate::response_resources::request::{
    CompactRequest, InputItemsOrder, ListInputItemsQuery, ResourceOperation, compact_request,
    list_input_items_request, resource_request,
};
use crate::response_resources::response::{decode_record, json_body, response_reply};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn provider_failure(error: &ResponseResourceError) -> (u16, String) {
    let ResponseResourceError::Provider(OpenAiError::Provider(provider)) = error else {
        panic!("expected an upstream failure, got {error:?}");
    };
    (provider.status_code, provider.message.clone())
}

fn reply_failure(result: Result<ResponsesReply, OpenAiError>) -> OpenAiError {
    match result {
        Ok(_reply) => panic!("expected the reply to fail"),
        Err(error) => error,
    }
}

fn payload_of(query: &ListInputItemsQuery) -> Value {
    let request = list_input_items_request("resp_1", "https://api.openai.com/v1", query).unwrap();
    serde_json::to_value(&request.payload).unwrap()
}

fn include_payload(include: Option<Vec<String>>) -> Value {
    let query = ListInputItemsQuery {
        after: None,
        before: None,
        include,
        limit: Some(20),
        order: Some(InputItemsOrder::Desc),
    };
    payload_of(&query)
}

fn input_of(text: &str) -> ResponsesInput {
    serde_json::from_str(text).unwrap()
}

fn options_of(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn compact_payload(
    model: &str,
    input: &ResponsesInput,
    options: &Object,
    provider: ProviderId,
    model_info: Option<&Object>,
) -> Value {
    let request = CompactRequest {
        model,
        input,
        options,
        provider,
        model_info,
    };
    let compacted = compact_request(&request, "https://api.openai.com/v1").unwrap();
    serde_json::to_value(&compacted.payload).unwrap()
}

fn lax_number(transformed: &ResponsesReply, name: &str) -> Json {
    match &transformed.response {
        ReplyBody::Lax(fields) => fields.get(name).cloned().unwrap_or(Json::Null),
        ReplyBody::Typed(_) => panic!("expected the reply to stay lax"),
    }
}

fn created_at_of(transformed: &ResponsesReply) -> i64 {
    match &transformed.response {
        ReplyBody::Typed(response) => response.created_at,
        ReplyBody::Lax(fields) => fields
            .get("created_at")
            .and_then(|given| serde_json::to_value(given).ok())
            .and_then(|given| given.as_i64())
            .unwrap_or_default(),
    }
}

pub(super) fn encoded_response_id() {
    encoded_paths();
    encoded_rejections();
}

fn encoded_paths() {
    let deleted = resource_request(
        "resp_1",
        "https://api.openai.com/v1",
        ResourceOperation::Delete,
    )
    .unwrap();
    assert_eq!(deleted.url, "https://api.openai.com/v1/resp_1");
    assert!(deleted.payload.is_empty());
    let retrieved = resource_request(
        "a/b c?d#e",
        "https://api.openai.com/v1",
        ResourceOperation::Get,
    )
    .unwrap();
    assert_eq!(retrieved.url, "https://api.openai.com/v1/a%2Fb%20c%3Fd%23e");
    assert!(retrieved.payload.is_empty());
    let cancelled = resource_request(
        "r\u{e9}s",
        "https://api.openai.com/v1",
        ResourceOperation::Cancel,
    )
    .unwrap();
    assert_eq!(cancelled.url, "https://api.openai.com/v1/r%C3%A9s/cancel");
    assert!(cancelled.payload.is_empty());
    let trailing = resource_request(
        "resp_2",
        "https://api.openai.com/v1/",
        ResourceOperation::Delete,
    )
    .unwrap();
    assert_eq!(trailing.url, "https://api.openai.com/v1//resp_2");
    let unreserved = resource_request(
        "x~y.z-_",
        "https://host.example/openai",
        ResourceOperation::Get,
    )
    .unwrap();
    assert_eq!(unreserved.url, "https://host.example/openai/x~y.z-_");
}

fn encoded_rejections() {
    let dot = resource_request("..", "https://api.openai.com/v1", ResourceOperation::Get);
    assert!(matches!(dot, Err(ResponseResourceError::ResponseId { .. })));
    assert_eq!(dot.unwrap_err().http_status(), None);
    let empty = resource_request("", "https://api.openai.com/v1", ResourceOperation::Delete);
    assert!(matches!(
        empty,
        Err(ResponseResourceError::ResponseId { .. })
    ));
}

pub(super) fn delete_result() {
    delete_values();
    delete_rejections();
}

fn delete_values() {
    let headers = IndexMap::new();
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_1","object":"response","deleted":true}"#,
    };
    let deleted = decode_record::<DeleteResponseResult>(&reply).unwrap();
    assert_eq!(deleted.id.as_deref(), Some("resp_1"));
    assert_eq!(deleted.object.as_deref(), Some("response"));
    assert_eq!(deleted.deleted, Some(true));
    let nullable = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":null,"object":null,"deleted":null}"#,
    };
    let empty = decode_record::<DeleteResponseResult>(&nullable).unwrap();
    assert_eq!(empty.id, None);
    assert_eq!(empty.object, None);
    assert_eq!(empty.deleted, None);
    let extra = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_2","object":"response","deleted":false,"vendor":{"n":1}}"#,
    };
    let kept = decode_record::<DeleteResponseResult>(&extra).unwrap();
    assert_eq!(kept.deleted, Some(false));
    assert_eq!(
        serde_json::to_value(kept.extra.get("vendor")).unwrap(),
        value(r#"{"n":1}"#)
    );
}

fn delete_rejections() {
    let headers = IndexMap::new();
    let missing = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_3","object":"response"}"#,
    };
    assert!(matches!(
        decode_record::<DeleteResponseResult>(&missing),
        Err(ResponseResourceError::Record { .. })
    ));
    let wrong = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":7,"object":"response","deleted":true}"#,
    };
    assert!(matches!(
        decode_record::<DeleteResponseResult>(&wrong),
        Err(ResponseResourceError::Record { .. })
    ));
    let list = RawResponse {
        status: 200,
        headers: &headers,
        text: "[]",
    };
    assert!(matches!(
        decode_record::<DeleteResponseResult>(&list),
        Err(ResponseResourceError::Record { .. })
    ));
    let broken = RawResponse {
        status: 503,
        headers: &headers,
        text: "not-json",
    };
    let error = decode_record::<DeleteResponseResult>(&broken).unwrap_err();
    assert_eq!(error.http_status(), Some(503));
    assert_eq!(provider_failure(&error), (503, String::from("not-json")));
}

pub(super) fn get_strict_response() {
    get_typed_reply();
    get_rejections();
    get_content_type();
    get_large_image_reply();
}

fn get_large_image_reply() {
    let headers = IndexMap::new();
    let image = "a".repeat(2 * 1024 * 1024);
    let body = format!(
        r#"{{"id":"resp_big","created_at":9,"output":[{{"type":"image_generation_call","id":"ig_1","result":"{image}","status":"completed"}}]}}"#
    );
    let typed = response_reply(&RawResponse {
        status: 200,
        headers: &headers,
        text: &body,
    })
    .unwrap();
    assert_eq!(typed.response.id, "resp_big");
}

fn get_typed_reply() {
    let headers = header_map(&[
        ("x-request-id", "req-1"),
        ("x-ratelimit-remaining-requests", "9"),
        ("llm_provider-x-custom", "keep"),
        ("x-test-header", "value"),
    ]);
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_1","created_at":9,"output":[],"status":"completed","model":"gpt-4o"}"#,
    };
    let typed = response_reply(&reply).unwrap();
    assert_eq!(typed.response.id, "resp_1");
    assert_eq!(typed.response.created_at, 9);
    assert_eq!(typed.response.status.as_deref(), Some("completed"));
    assert_eq!(typed.response.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        serde_json::to_value(&typed.headers).unwrap(),
        value(
            r#"{"x-request-id":"req-1","x-ratelimit-remaining-requests":"9","llm_provider-x-custom":"keep","x-test-header":"value"}"#
        )
    );
    assert_eq!(
        serde_json::to_value(&typed.additional_headers).unwrap(),
        value(
            r#"{"x-ratelimit-remaining-requests":"9","llm_provider-x-custom":"keep","llm_provider-x-request-id":"req-1","llm_provider-x-ratelimit-remaining-requests":"9","llm_provider-x-test-header":"value"}"#
        )
    );
    let minimal = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_2","created_at":5,"output":[],"store":true}"#,
    };
    let plain = response_reply(&minimal).unwrap();
    assert_eq!(plain.response.status, None);
    assert_eq!(plain.response.model, None);
}

fn get_rejections() {
    let headers = IndexMap::new();
    let lax_only = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":7,"created_at":5,"output":"bad"}"#,
    };
    assert!(matches!(
        response_reply(&lax_only),
        Err(ResponseResourceError::Record { .. })
    ));
    let text_timestamp = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_3","created_at":"3.9","output":[]}"#,
    };
    assert!(matches!(
        response_reply(&text_timestamp),
        Err(ResponseResourceError::Record { .. })
    ));
    let list = RawResponse {
        status: 200,
        headers: &headers,
        text: "[]",
    };
    assert!(matches!(
        response_reply(&list),
        Err(ResponseResourceError::Record { .. })
    ));
    let broken = RawResponse {
        status: 400,
        headers: &headers,
        text: "{oops",
    };
    let error = response_reply(&broken).unwrap_err();
    assert_eq!(provider_failure(&error), (400, String::from("{oops")));
}

fn get_content_type() {
    let content_type = header_map(&[("content-type", "application/json")]);
    let progress = RawResponse {
        status: 200,
        headers: &content_type,
        text: r#"{"id":"resp_4","created_at":9,"output":[],"status":"in_progress"}"#,
    };
    let typed = response_reply(&progress).unwrap();
    assert_eq!(
        serde_json::to_value(&typed.additional_headers).unwrap(),
        value(r#"{"llm_provider-content-type":"application/json"}"#)
    );
}

pub(super) fn list_default_limit_order() {
    list_defaults();
    list_explicit();
    list_omissions();
}

fn list_defaults() {
    assert_eq!(
        payload_of(&ListInputItemsQuery::default()),
        value(r#"{"limit":20,"order":"desc"}"#)
    );
}

fn list_explicit() {
    let explicit = ListInputItemsQuery {
        after: None,
        before: None,
        include: None,
        limit: Some(5),
        order: Some(InputItemsOrder::Asc),
    };
    assert_eq!(payload_of(&explicit), value(r#"{"limit":5,"order":"asc"}"#));
    let empty_strings = ListInputItemsQuery {
        after: Some(String::new()),
        before: Some(String::new()),
        include: None,
        limit: Some(0),
        order: Some(InputItemsOrder::Desc),
    };
    assert_eq!(
        payload_of(&empty_strings),
        value(r#"{"after":"","before":"","limit":0,"order":"desc"}"#)
    );
}

fn list_omissions() {
    let omitted = ListInputItemsQuery {
        after: None,
        before: None,
        include: None,
        limit: None,
        order: None,
    };
    assert_eq!(payload_of(&omitted), value("{}"));
    let items = ListInputItemsQuery {
        after: Some(String::from("item_1")),
        before: Some(String::from("item_9")),
        include: None,
        limit: None,
        order: None,
    };
    let item_request =
        list_input_items_request("a/b", "https://api.openai.com/v1", &items).unwrap();
    assert_eq!(
        item_request.url,
        "https://api.openai.com/v1/a%2Fb/input_items"
    );
    assert_eq!(
        serde_json::to_value(&item_request.payload).unwrap(),
        value(r#"{"after":"item_1","before":"item_9"}"#)
    );
    let trailing = ListInputItemsQuery {
        after: None,
        before: None,
        include: None,
        limit: Some(5),
        order: Some(InputItemsOrder::Asc),
    };
    let trailing_request =
        list_input_items_request("resp_1", "https://api.openai.com/v1/", &trailing).unwrap();
    assert_eq!(
        trailing_request.url,
        "https://api.openai.com/v1//resp_1/input_items"
    );
}

pub(super) fn include_comma_query() {
    assert_eq!(
        include_payload(Some(vec![
            String::from("input_text"),
            String::from("output_text"),
        ])),
        value(r#"{"include":"input_text,output_text","limit":20,"order":"desc"}"#)
    );
    assert_eq!(
        include_payload(Some(vec![String::new()])),
        value(r#"{"include":"","limit":20,"order":"desc"}"#)
    );
    assert_eq!(
        include_payload(Some(vec![String::from("a"), String::new()])),
        value(r#"{"include":"a,","limit":20,"order":"desc"}"#)
    );
    assert_eq!(
        include_payload(Some(Vec::new())),
        value(r#"{"limit":20,"order":"desc"}"#)
    );
    assert_eq!(
        include_payload(None),
        value(r#"{"limit":20,"order":"desc"}"#)
    );
}

pub(super) fn cancel_response() {
    cancel_value();
    cancel_rejections();
}

fn cancel_value() {
    let headers = header_map(&[("x-request-id", "req-1")]);
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_1","created_at":9,"output":[],"status":"cancelled"}"#,
    };
    let typed = response_reply(&reply).unwrap();
    assert_eq!(typed.response.status.as_deref(), Some("cancelled"));
    assert_eq!(
        serde_json::to_value(&typed.additional_headers).unwrap(),
        value(r#"{"llm_provider-x-request-id":"req-1"}"#)
    );
}

fn cancel_rejections() {
    let headers = IndexMap::new();
    let lax_only = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"id":"resp_2","created_at":5,"output":"bad"}"#,
    };
    assert!(matches!(
        response_reply(&lax_only),
        Err(ResponseResourceError::Record { .. })
    ));
    let broken = RawResponse {
        status: 502,
        headers: &headers,
        text: "not json",
    };
    let error = response_reply(&broken).unwrap_err();
    assert_eq!(provider_failure(&error), (502, String::from("not json")));
}

pub(super) fn compact_keeps_query() {
    compact_paths();
    compact_options();
}

fn compact_paths() {
    let input = ResponsesInput::Text(String::from("hi"));
    let options = Object::new();
    let request = CompactRequest {
        model: "gpt-4o",
        input: &input,
        options: &options,
        provider: ProviderId::Openai,
        model_info: None,
    };
    let plain = compact_request(&request, "https://api.openai.com/v1").unwrap();
    assert_eq!(plain.url, "https://api.openai.com/v1/compact");
    assert_eq!(
        serde_json::to_value(&plain.payload).unwrap(),
        value(r#"{"model":"gpt-4o","input":"hi"}"#)
    );
    let slashed = compact_request(&request, "https://api.openai.com/v1/").unwrap();
    assert_eq!(slashed.url, "https://api.openai.com/v1/compact");
    let versioned =
        compact_request(&request, "https://api.openai.com/v1?api-version=2025-01-01").unwrap();
    assert_eq!(
        versioned.url,
        "https://api.openai.com/v1/compact?api-version=2025-01-01"
    );
    let bare = compact_request(&request, "https://api.openai.com").unwrap();
    assert_eq!(bare.url, "https://api.openai.com/compact");
    let doubled = compact_request(&request, "https://host/v1//").unwrap();
    assert_eq!(doubled.url, "https://host/v1/compact");
    let escaped = compact_request(&request, "https://host/a%2Fb").unwrap();
    assert_eq!(escaped.url, "https://host/a%2Fb/compact");
}

fn compact_options() {
    let input = ResponsesInput::Text(String::from("hi"));
    let options = options_of(r#"{"temperature":1}"#);
    let request = CompactRequest {
        model: "gpt-4o",
        input: &input,
        options: &options,
        provider: ProviderId::Openai,
        model_info: None,
    };
    let compacted = compact_request(
        &request,
        "https://host.example/openai/v1/?api-version=2025-01-01&x=1",
    )
    .unwrap();
    assert_eq!(
        compacted.url,
        "https://host.example/openai/v1/compact?api-version=2025-01-01&x=1"
    );
    assert_eq!(
        serde_json::to_value(&compacted.payload).unwrap(),
        value(r#"{"model":"gpt-4o","input":"hi","temperature":1}"#)
    );
    let error = compact_request(&request, "not a url").unwrap_err();
    assert!(matches!(error, ResponseResourceError::ApiBase { .. }));
    assert_eq!(error.http_status(), None);
}

const REASONING_INPUT: &str = r#"[{"type":"reasoning","id":"rs_1","summary":[],"status":null,"content":null,"encrypted_content":null},{"role":"user","content":[{"type":"input_text","text":"hi","cache_control":{"type":"ephemeral"}}]}]"#;
const REASONING_TOOLS: &str = r#"{"tools":[{"type":"function","name":"f","parameters":"{\"type\":\"object\"}","cache_control":{"type":"ephemeral"}}]}"#;
const REASONING_EXPECTED: &str = r#"{"model":"gpt-4o","input":[{"id":"rs_1","summary":[],"type":"reasoning"},{"role":"user","content":[{"type":"input_text","text":"hi"}]}],"tools":[{"type":"function","name":"f","parameters":{"type":"object"}}]}"#;
const FOREIGN_INPUT: &str = r#"[{"type":"function_call","id":"fc_1","call_id":"c","name":"f","arguments":"{}"},{"type":"function_call","id":"other_1","call_id":"c","name":"f","arguments":"{}"},{"type":"custom_tool_call","id":"ctc_1","call_id":"c","name":"f","input":"x"},{"type":"custom_tool_call","id":"foreign","call_id":"c","name":"f","input":"x"}]"#;
const FOREIGN_EXPECTED: &str = r#"{"model":"gpt-4o","input":[{"type":"function_call","id":"fc_1","call_id":"c","name":"f","arguments":"{}"},{"type":"function_call","call_id":"c","name":"f","arguments":"{}"},{"type":"custom_tool_call","id":"ctc_1","call_id":"c","name":"f","input":"x"},{"type":"custom_tool_call","call_id":"c","name":"f","input":"x"}]}"#;
const VLLM_INPUT: &str =
    r#"[{"type":"function_call","id":"other_1","call_id":"c","name":"f","arguments":"{}"}]"#;
const VLLM_TOOLS: &str =
    r#"{"tools":[{"type":"function","name":"f","parameters":{"allOf":[{"type":"object"}]}}]}"#;
const VLLM_EXPECTED: &str = r#"{"model":"gpt-4o","input":[{"type":"function_call","id":"other_1","call_id":"c","name":"f","arguments":"{}"}],"tools":[{"type":"function","name":"f","parameters":{"allOf":[{"type":"object"}]}}]}"#;
const SCHEMA_TOOLS: &str = r#"{"tools":[{"type":"function","name":"f","parameters":{"type":"object","anyOf":[{"type":"string"}],"properties":{"bad":{"pattern":"("},"good":{"pattern":"^a+$"}}}}]}"#;
const SCHEMA_EXPECTED: &str = r#"{"model":"gpt-4o","input":"hi","tools":[{"type":"function","name":"f","parameters":{"type":"object","properties":{"bad":{},"good":{"pattern":"^a+$"}}}}]}"#;
const NEUTRAL_TOOLS: &str = r#"{"tools":null}"#;
const NEUTRAL_EXPECTED: &str = r#"{"model":"gpt-4o","input":"hi","tools":null}"#;

fn compacted(input: &str, options: &str, provider: ProviderId) -> Value {
    let request_input = input_of(input);
    let request_options = options_of(options);
    compact_payload("gpt-4o", &request_input, &request_options, provider, None)
}

pub(super) fn compact_replay_input() {
    replay_reasoning_and_tools();
    replay_foreign_ids();
    replay_provider_gates();
    replay_schema_gate();
    replay_null_tools();
    replay_invalid_tools();
}

fn replay_invalid_tools() {
    let input = ResponsesInput::Text(String::from("hi"));
    let options =
        options_of(r#"{"tools":[{"type":"function","name":"f","parameters":"not json"}]}"#);
    let request = CompactRequest {
        model: "gpt-4o",
        input: &input,
        options: &options,
        provider: ProviderId::Openai,
        model_info: None,
    };
    let error = compact_request(&request, "https://api.openai.com/v1").unwrap_err();
    assert!(matches!(error, ResponseResourceError::Input(_)));
    assert_eq!(error.http_status(), Some(400));
}

fn replay_reasoning_and_tools() {
    assert_eq!(
        compacted(REASONING_INPUT, REASONING_TOOLS, ProviderId::Openai),
        value(REASONING_EXPECTED)
    );
}

fn replay_foreign_ids() {
    assert_eq!(
        compacted(FOREIGN_INPUT, "{}", ProviderId::Openai),
        value(FOREIGN_EXPECTED)
    );
}

fn replay_provider_gates() {
    assert_eq!(
        compacted(VLLM_INPUT, VLLM_TOOLS, ProviderId::HostedVllm),
        value(VLLM_EXPECTED)
    );
}

fn replay_schema_gate() {
    let model_info = options_of(r#"{"base_model":"gpt-4o"}"#);
    assert_eq!(
        compact_payload(
            "gpt-4o",
            &input_of(r#""hi""#),
            &options_of(SCHEMA_TOOLS),
            ProviderId::Openai,
            Some(&model_info),
        ),
        value(SCHEMA_EXPECTED)
    );
}

fn replay_null_tools() {
    assert_eq!(
        compacted(r#""hi""#, NEUTRAL_TOOLS, ProviderId::Openai),
        value(NEUTRAL_EXPECTED)
    );
}

pub(super) fn compact_lax_response() {
    lax_typed_reply();
    lax_fallback();
    lax_failure();
}

fn lax_typed_reply() {
    let headers = header_map(&[
        ("x-request-id", "req-1"),
        ("x-ratelimit-remaining-requests", "9"),
        ("llm_provider-x-custom", "keep"),
        ("x-test-header", "value"),
    ]);
    let (result, state) = compact_call(
        r#"{"id":"resp_1","created_at":9,"output":[],"status":"completed"}"#,
        200,
        &headers,
        1_700_000_000,
    );
    let transformed = result.unwrap();
    match &transformed.response {
        ReplyBody::Typed(response) => {
            assert_eq!(response.id, "resp_1");
            assert_eq!(response.created_at, 9);
        }
        ReplyBody::Lax(fields) => panic!("expected a typed reply, got {fields:?}"),
    }
    assert_eq!(
        serde_json::to_value(&transformed.additional_headers).unwrap(),
        value(
            r#"{"x-ratelimit-remaining-requests":"9","llm_provider-x-custom":"keep","llm_provider-x-request-id":"req-1","llm_provider-x-ratelimit-remaining-requests":"9","llm_provider-x-test-header":"value"}"#
        )
    );
    assert_eq!(
        serde_json::to_value(state.model_call_details.get("original_response")).unwrap(),
        value(r#""{\"id\":\"resp_1\",\"created_at\":9,\"output\":[],\"status\":\"completed\"}""#)
    );
    assert_eq!(
        serde_json::to_value(state.model_call_details.get("additional_args")).unwrap(),
        value(r#"{"complete_input_dict":{}}"#)
    );
}

fn lax_fallback() {
    let headers = IndexMap::new();
    let (result, _state) = compact_call(
        r#"{"id":7,"created_at":5,"output":"not-a-list"}"#,
        200,
        &headers,
        1_700_000_000,
    );
    let transformed = result.unwrap();
    let Json::Number(number) = lax_number(&transformed, "id") else {
        panic!("the lax id must stay the number LiteLLM kept");
    };
    assert_eq!(number.as_i64(), Some(7));
    let (converted, _converted_state) = compact_call(
        r#"{"id":"resp_3","created_at":"3.9","output":[],"vendor_extra":{"n":1}}"#,
        200,
        &headers,
        1_700_000_000,
    );
    assert_eq!(created_at_of(&converted.unwrap()), 3);
    let (current, _current_state) = compact_call(
        r#"{"id":"resp_4","created_at":"not-a-time","output":[]}"#,
        200,
        &headers,
        1_700_000_000,
    );
    assert_eq!(created_at_of(&current.unwrap()), 1_700_000_000);
}

fn compact_failure(text: &str, status: u16) -> (u16, String) {
    let headers = IndexMap::new();
    let (result, _state) = compact_call(text, status, &headers, 1_700_000_000);
    let OpenAiError::Provider(provider) = reply_failure(result) else {
        panic!("expected an upstream failure");
    };
    (provider.status_code, provider.message)
}

fn lax_failure() {
    assert_eq!(
        compact_failure("not json", 503),
        (503, String::from("not json"))
    );
}

pub(super) fn resource_json_error_status() {
    json_error_statuses();
    list_without_validation();
}

fn json_error_statuses() {
    let headers = IndexMap::new();
    let delete_broken = RawResponse {
        status: 400,
        headers: &headers,
        text: "not json",
    };
    let delete_error = decode_record::<DeleteResponseResult>(&delete_broken).unwrap_err();
    assert_eq!(
        provider_failure(&delete_error),
        (400, String::from("not json"))
    );
    let get_broken = RawResponse {
        status: 503,
        headers: &headers,
        text: r#"{"id":"#,
    };
    let get_error = response_reply(&get_broken).unwrap_err();
    assert_eq!(
        provider_failure(&get_error),
        (503, String::from(r#"{"id":"#))
    );
    let list_broken = RawResponse {
        status: 401,
        headers: &headers,
        text: "not json",
    };
    let list_error = json_body(&list_broken).unwrap_err();
    assert_eq!(
        provider_failure(&list_error),
        (401, String::from("not json"))
    );
    cancel_json_failure();
    compact_json_failure();
}

fn cancel_json_failure() {
    let headers = IndexMap::new();
    let cancel_broken = RawResponse {
        status: 500,
        headers: &headers,
        text: "{}x",
    };
    let cancel_error = response_reply(&cancel_broken).unwrap_err();
    assert_eq!(provider_failure(&cancel_error), (500, String::from("{}x")));
}

fn compact_json_failure() {
    assert_eq!(compact_failure("[]", 429), (429, String::from("[]")));
}

fn list_without_validation() {
    let headers = IndexMap::new();
    let items = RawResponse {
        status: 200,
        headers: &headers,
        text: "[1,2,3]",
    };
    assert_eq!(
        serde_json::to_value(json_body(&items).unwrap()).unwrap(),
        value("[1,2,3]")
    );
    let page = RawResponse {
        status: 200,
        headers: &headers,
        text: r#"{"data":[{"id":"item_1"}],"has_more":false}"#,
    };
    assert_eq!(
        serde_json::to_value(json_body(&page).unwrap()).unwrap(),
        value(r#"{"data":[{"id":"item_1"}],"has_more":false}"#)
    );
}
