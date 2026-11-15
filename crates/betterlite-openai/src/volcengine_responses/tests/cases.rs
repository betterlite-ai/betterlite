use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;

use betterlite_models::decisions::Json;

use crate::openai_base_config::{Headers, Object};
use crate::response_events::event_decode::DecodedStreamResponse;
use crate::response_events::{RawResponse, ReplyBody, ResponsesReply};
use crate::volcengine_common::VolcEngineError;
use crate::volcengine_responses::error::VolcengineResponsesError;
use crate::volcengine_responses::params::{
    map_openai_params, sanitize_request, supported_openai_params,
};
use crate::volcengine_responses::reply::{
    ReplyOperation, ResourceReply, transform_resource_reply, transform_response,
};
use crate::volcengine_responses::request::{
    BaseSources, KeySources, get_complete_url, validate_environment,
};
use crate::volcengine_responses::stream::transform_streaming_response;
use crate::volcengine_responses::{SHOULD_FAKE_STREAM, SUPPORTS_NATIVE_WEBSOCKET};

use super::support;

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn event(text: &str) -> IndexMap<String, Json> {
    let parsed: Json = serde_json::from_str(text).unwrap();
    match parsed {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object event")
        }
    }
}

fn value(decoded: &DecodedStreamResponse) -> Value {
    match decoded {
        DecodedStreamResponse::Known { record, .. } => serde_json::to_value(record).unwrap(),
        DecodedStreamResponse::Generic(generic) => serde_json::to_value(generic).unwrap(),
        DecodedStreamResponse::Lax { fields, .. } => {
            serde_json::to_value(Json::Object(fields.clone())).unwrap()
        }
    }
}

fn stream(text: &str) -> Value {
    let decoded = transform_streaming_response(&event(text)).unwrap();
    value(&decoded)
}

fn stream_error(text: &str) -> VolcengineResponsesError {
    match transform_streaming_response(&event(text)) {
        Ok(_decoded) => panic!("expected the event to fail"),
        Err(error) => error,
    }
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn wire_reply<'text>(
    text: &'text str,
    status: u16,
    headers: &'text IndexMap<String, String>,
) -> RawResponse<'text> {
    RawResponse {
        status,
        headers,
        text,
    }
}

fn transformed(text: &str, status: u16) -> Result<ResponsesReply, VolcEngineError> {
    let redactor = support::redactor();
    let silent = support::Silent;
    let env = support::environment(&redactor, &silent);
    let mut state = support::logging(&redactor);
    let headers = IndexMap::new();
    let reply = wire_reply(text, status, &headers);
    transform_response(&reply, support::NOW_SECONDS, &mut state, &env)
}

#[test]
fn supported_params_lists_the_documented_options_without_metadata() {
    let supported = supported_openai_params();
    assert_eq!(supported.first(), Some(&"input"));
    assert_eq!(supported.get(1), Some(&"model"));
    // input and model plus the twenty optional keys, without metadata
    assert_eq!(supported.len(), 22);
    assert!(!supported.contains(&"metadata"));
    assert!(supported.contains(&"thinking"));
    assert!(supported.contains(&"timeout"));
}

#[test]
fn map_params_drops_metadata_parallel_tool_calls_and_unknown_keys() {
    let mapped = map_openai_params(&object(
        r#"{"instructions":"x","metadata":{"a":1},"parallel_tool_calls":true,"unknown":1,"store":false}"#,
    ));
    assert_eq!(mapped, object(r#"{"instructions":"x","store":false}"#));
}

#[test]
fn map_params_logs_the_dropped_parallel_tool_calls() {
    let (mapped, logs) = support::capture(|| {
        map_openai_params(&object(r#"{"parallel_tool_calls":true,"store":true}"#))
    });
    assert_eq!(mapped, object(r#"{"store":true}"#));
    assert!(logs.contains("dropping unsupported 'parallel_tool_calls' param"));
    let (_quiet, quiet_logs) = support::capture(|| map_openai_params(&object(r#"{"store":true}"#)));
    assert!(!quiet_logs.contains("parallel_tool_calls"));
}

#[test]
fn sanitize_request_filters_extra_body_keys_and_drops_an_empty_body() {
    let filtered = sanitize_request(&object(
        r#"{"metadata":1,"extra_body":{"instructions":"y","input":2,"model":3},"temperature":0}"#,
    ));
    assert_eq!(
        filtered,
        object(r#"{"extra_body":{"instructions":"y"},"temperature":0}"#)
    );
    let emptied = sanitize_request(&object(r#"{"extra_body":{"input":1,"model":2}}"#));
    assert_eq!(emptied, object("{}"));
    let untouched = sanitize_request(&object(r#"{"extra_body":5}"#));
    assert_eq!(untouched, object(r#"{"extra_body":5}"#));
}

#[test]
fn key_lookup_takes_the_first_nonempty_source_in_order() {
    let params = secret("params-key");
    let global = secret("");
    let ark = secret("ark-key");
    let volcengine = secret("volc-key");
    let sources = KeySources {
        litellm_params: Some(&params),
        global: Some(&global),
        ark: Some(&ark),
        volcengine: Some(&volcengine),
    };
    let headers = validate_environment(&Headers::new(), sources).unwrap();
    assert_eq!(&headers["Authorization"], "Bearer params-key");
}

#[test]
fn key_lookup_falls_back_to_the_volcengine_key() {
    let volcengine = secret("volc-key");
    let sources = KeySources {
        litellm_params: None,
        global: None,
        ark: None,
        volcengine: Some(&volcengine),
    };
    let headers = validate_environment(&Headers::new(), sources).unwrap();
    assert_eq!(&headers["Authorization"], "Bearer volc-key");
}

#[test]
fn key_lookup_fails_without_a_key() {
    let empty = secret("");
    let sources = KeySources {
        litellm_params: Some(&empty),
        global: None,
        ark: None,
        volcengine: None,
    };
    assert!(matches!(
        validate_environment(&Headers::new(), sources),
        Err(VolcengineResponsesError::MissingApiKey)
    ));
}

#[test]
fn environment_headers_put_the_volcengine_defaults_first() {
    let key = secret("volc-key");
    let extra = Headers::from([(String::from("Authorization"), String::from("Custom"))]);
    let sources = KeySources {
        litellm_params: Some(&key),
        global: None,
        ark: None,
        volcengine: None,
    };
    let headers = validate_environment(&extra, sources).unwrap();
    assert_eq!(&headers["Content-Type"], "application/json");
    assert_eq!(&headers["Authorization"], "Custom");
}

const NO_SOURCES: BaseSources<'static> = BaseSources {
    api_base: None,
    global: None,
    volcengine: None,
    ark: None,
};

#[test]
fn complete_url_falls_back_to_the_default() {
    assert_eq!(
        get_complete_url(NO_SOURCES),
        "https://ark.cn-beijing.volces.com/api/v3/responses"
    );
}

#[test]
fn complete_url_keeps_a_responses_base() {
    let sources = BaseSources {
        api_base: Some("https://ark.example.test/responses/"),
        global: None,
        volcengine: None,
        ark: None,
    };
    assert_eq!(
        get_complete_url(sources),
        "https://ark.example.test/responses"
    );
}

#[test]
fn complete_url_appends_the_version_for_an_api_v3_base() {
    let sources = BaseSources {
        api_base: Some("https://ark.example.test/api/v3"),
        global: None,
        volcengine: None,
        ark: None,
    };
    assert_eq!(
        get_complete_url(sources),
        "https://ark.example.test/api/v3/responses"
    );
}

#[test]
fn complete_url_reads_the_sources_in_order() {
    let sources = BaseSources {
        api_base: Some(""),
        global: Some("https://global.test"),
        volcengine: Some("https://volc.test"),
        ark: Some("https://ark.test"),
    };
    assert_eq!(
        get_complete_url(sources),
        "https://global.test/api/v3/responses"
    );
    let env_first = BaseSources {
        api_base: None,
        global: None,
        volcengine: Some("https://volc.test"),
        ark: Some("https://ark.test"),
    };
    assert_eq!(
        get_complete_url(env_first),
        "https://volc.test/api/v3/responses"
    );
    let ark_last = BaseSources {
        api_base: None,
        global: None,
        volcengine: None,
        ark: Some("https://ark.test"),
    };
    assert_eq!(
        get_complete_url(ark_last),
        "https://ark.test/api/v3/responses"
    );
}

#[test]
fn stream_event_adds_an_empty_output_to_a_response() {
    let decoded =
        stream(r#"{"type":"response.created","response":{"id":"resp_1","created_at":7}}"#);
    assert_eq!(decoded["type"], "response.created");
    assert_eq!(decoded["response"]["id"], "resp_1");
    assert_eq!(decoded["response"]["output"], serde_json::json!([]));
}

#[test]
fn stream_event_fills_missing_integers_with_zero() {
    let decoded = stream(r#"{"type":"response.output_text.delta","item_id":"i","delta":"d"}"#);
    assert_eq!(decoded["output_index"], 0_i32);
    assert_eq!(decoded["content_index"], 0_i32);
}

#[test]
fn stream_event_fills_a_list_with_an_empty_list() {
    let decoded = stream(
        r#"{"type":"response.content_part.done","item_id":"i","output_index":1,"content_index":0,"part":{"type":"output_text","text":"hi"}}"#,
    );
    assert_eq!(decoded["part"]["annotations"], serde_json::json!([]));
    assert_eq!(decoded["part"]["logprobs"], serde_json::json!([]));
}

#[test]
fn stream_event_fills_a_refusal_part() {
    let decoded = stream(
        r#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":0,"part":{"type":"refusal","refusal":"no"}}"#,
    );
    assert_eq!(decoded["part"]["refusal"], "no");
    assert!(decoded["part"].get("text").is_none());
}

#[test]
fn stream_event_fills_a_reasoning_part() {
    let decoded = stream(
        r#"{"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":0,"part":{"type":"reasoning_text","reasoning":"why"}}"#,
    );
    assert_eq!(decoded["part"]["reasoning"], "why");
    assert!(decoded["part"].get("text").is_none());
}

#[test]
fn stream_event_fills_a_nullable_field_with_null() {
    let decoded = stream(r#"{"type":"response.output_item.added"}"#);
    assert_eq!(decoded["output_index"], 0_i32);
    assert!(decoded.get("item").is_none() || decoded["item"].is_null());
}

#[test]
fn stream_event_recurses_into_a_nested_usage() {
    let decoded = stream(
        r#"{"type":"response.completed","response":{"id":"r","created_at":1,"output":[],"usage":{"input_tokens":3}}}"#,
    );
    assert_eq!(decoded["response"]["usage"]["input_tokens"], 3_i32);
    assert_eq!(decoded["response"]["usage"]["output_tokens"], 0_i32);
    assert_eq!(decoded["response"]["usage"]["total_tokens"], 0_i32);
}

#[rstest]
#[case::content_part_added(r#"{"type":"response.content_part.added","item_id":"i","part":{"type":"output_text"}}"#, &["output_index", "content_index"])]
#[case::output_text_done(r#"{"type":"response.output_text.done","item_id":"i","text":"t"}"#, &["output_index", "content_index"])]
#[case::refusal_delta(r#"{"type":"response.refusal.delta","item_id":"i","delta":"d"}"#, &["output_index", "content_index"])]
#[case::refusal_done(r#"{"type":"response.refusal.done","item_id":"i","refusal":"r"}"#, &["output_index", "content_index"])]
#[case::annotation_added(r#"{"type":"response.output_text.annotation.added","item_id":"i","annotation":{"type":"url_citation"}}"#, &["output_index", "content_index", "annotation_index"])]
#[case::function_delta(r#"{"type":"response.function_call_arguments.delta","item_id":"i","delta":"d"}"#, &["output_index"])]
#[case::function_done(r#"{"type":"response.function_call_arguments.done","item_id":"i","arguments":"{}"}"#, &["output_index"])]
#[case::output_item_done(r#"{"type":"response.output_item.done","item":{"type":"message"}}"#, &["output_index"])]
#[case::file_search_call_in_progress(r#"{"type":"response.file_search_call.in_progress","item_id":"i"}"#, &["output_index"])]
#[case::file_search_call_searching(r#"{"type":"response.file_search_call.searching","item_id":"i"}"#, &["output_index"])]
#[case::file_search_call_completed(r#"{"type":"response.file_search_call.completed","item_id":"i"}"#, &["output_index"])]
#[case::web_search_call_in_progress(r#"{"type":"response.web_search_call.in_progress","item_id":"i"}"#, &["output_index"])]
#[case::web_search_call_searching(r#"{"type":"response.web_search_call.searching","item_id":"i"}"#, &["output_index"])]
#[case::web_search_call_completed(r#"{"type":"response.web_search_call.completed","item_id":"i"}"#, &["output_index"])]
#[case::mcp_list_tools_in_progress(r#"{"type":"response.mcp_list_tools.in_progress","item_id":"i"}"#, &["sequence_number", "output_index"])]
#[case::mcp_list_tools_completed(r#"{"type":"response.mcp_list_tools.completed","item_id":"i"}"#, &["sequence_number", "output_index"])]
#[case::mcp_list_tools_failed(r#"{"type":"response.mcp_list_tools.failed","item_id":"i"}"#, &["sequence_number", "output_index"])]
#[case::mcp_call_in_progress(r#"{"type":"response.mcp_call.in_progress","item_id":"i"}"#, &["sequence_number", "output_index"])]
#[case::mcp_call_completed(r#"{"type":"response.mcp_call.completed","item_id":"i"}"#, &["sequence_number", "output_index"])]
#[case::mcp_call_failed(r#"{"type":"response.mcp_call.failed","item_id":"i"}"#, &["sequence_number", "output_index"])]
#[case::mcp_call_arguments_delta(r#"{"type":"response.mcp_call_arguments.delta","item_id":"i","delta":"d"}"#, &["output_index", "sequence_number"])]
#[case::mcp_call_arguments_done(r#"{"type":"response.mcp_call_arguments.done","item_id":"i","arguments":"{}"}"#, &["output_index", "sequence_number"])]
#[case::image_partial(r#"{"type":"image_generation.partial_image","b64_json":"aGk="}"#, &["partial_image_index"])]
#[case::error(r#"{"type":"error","error":{"type":"invalid_request_error","code":"c","message":"m"}}"#, &["sequence_number"])]
fn stream_event_fills_every_integer_its_event_declares(
    #[case] text: &str,
    #[case] integers: &[&str],
) {
    let decoded = stream(text);
    for name in integers {
        assert_eq!(decoded[*name], 0_i32, "{name}");
    }
}

#[test]
fn stream_event_gives_an_output_item_done_its_declared_default_sequence() {
    let decoded = stream(r#"{"type":"response.output_item.done","item":{"type":"message"}}"#);
    assert_eq!(decoded["sequence_number"], 1_i32);
}

#[test]
fn stream_event_rejects_an_error_event_without_a_string_code() {
    assert!(matches!(
        stream_error(
            r#"{"type":"error","error":{"type":"invalid_request_error","code":null,"message":"m"}}"#
        ),
        VolcengineResponsesError::EventRecord { .. }
    ));
}

#[test]
fn stream_event_rejects_an_event_without_a_string_type() {
    assert!(matches!(
        stream_error(r#"{"item_id":"i"}"#),
        VolcengineResponsesError::EventType
    ));
    assert!(matches!(
        stream_error(r#"{"type":7}"#),
        VolcengineResponsesError::EventType
    ));
}

#[test]
fn stream_event_keeps_an_unknown_event_generic() {
    let decoded = stream(r#"{"type":"response.custom.thing","extra":1}"#);
    assert_eq!(decoded["type"], "response.custom.thing");
    assert_eq!(decoded["extra"], 1_i32);
}

#[test]
fn stream_event_rejects_an_event_that_still_does_not_validate() {
    assert!(matches!(
        stream_error(
            r#"{"type":"response.output_text.delta","item_id":"i","delta":"d","output_index":"x"}"#
        ),
        VolcengineResponsesError::EventRecord { .. }
    ));
}

#[test]
fn reply_converts_created_at_to_an_integer() {
    let reply = transformed(r#"{"id":"resp_1","created_at":"7.9","output":[]}"#, 200).unwrap();
    match reply.response {
        ReplyBody::Typed(response) => assert_eq!(response.created_at, 7),
        ReplyBody::Lax(_fields) => panic!("expected a typed reply"),
    }
}

#[test]
fn reply_keeps_a_reply_that_does_not_validate() {
    let reply = transformed(r#"{"id":"resp_1","created_at":5,"output":"bad"}"#, 200).unwrap();
    assert!(matches!(reply.response, ReplyBody::Lax(_fields)));
}

#[test]
fn reply_reports_a_malformed_body_as_its_status() {
    let Err(error) = transformed("not-json", 503) else {
        panic!("a body that is not JSON must fail");
    };
    assert_eq!(error.status_code, 503);
    assert_eq!(error.message, "not-json");
}

#[test]
fn get_reply_validates_strictly() {
    let headers = IndexMap::new();
    let reply = wire_reply(
        r#"{"id":"resp_1","created_at":9,"output":[]}"#,
        200,
        &headers,
    );
    match transform_resource_reply(ReplyOperation::Get, &reply).unwrap() {
        ResourceReply::Response(typed) => assert_eq!(typed.response.id, "resp_1"),
        ResourceReply::Body(_body) => panic!("expected a typed response"),
    }
    let broken = wire_reply(r#"{"id":7,"created_at":9,"output":[]}"#, 200, &headers);
    assert!(transform_resource_reply(ReplyOperation::Get, &broken).is_err());
}

#[test]
fn delete_reply_keeps_the_fields_as_sent_when_validation_fails() {
    let headers = IndexMap::new();
    let reply = wire_reply(r#"{"deleted":true}"#, 200, &headers);
    match transform_resource_reply(ReplyOperation::Delete, &reply).unwrap() {
        ResourceReply::Body(body) => assert_eq!(
            serde_json::to_value(*body).unwrap(),
            json(r#"{"deleted":true}"#)
        ),
        ResourceReply::Response(_typed) => panic!("expected a body"),
    }
}

#[test]
fn delete_reply_returns_the_validated_record() {
    let headers = IndexMap::new();
    let reply = wire_reply(
        r#"{"id":"resp_1","object":"response","deleted":true,"extra":1}"#,
        200,
        &headers,
    );
    match transform_resource_reply(ReplyOperation::Delete, &reply).unwrap() {
        ResourceReply::Body(body) => assert_eq!(
            serde_json::to_value(*body).unwrap(),
            json(r#"{"id":"resp_1","object":"response","deleted":true,"extra":1}"#)
        ),
        ResourceReply::Response(_typed) => panic!("expected a body"),
    }
}

#[test]
fn input_items_reply_stays_raw() {
    let headers = IndexMap::new();
    let reply = wire_reply(r#"{"data":[],"object":"list"}"#, 200, &headers);
    match transform_resource_reply(ReplyOperation::InputItems, &reply).unwrap() {
        ResourceReply::Body(body) => assert_eq!(
            serde_json::to_value(*body).unwrap(),
            json(r#"{"data":[],"object":"list"}"#)
        ),
        ResourceReply::Response(_typed) => panic!("expected a body"),
    }
    let broken = wire_reply("not-json", 500, &headers);
    assert!(transform_resource_reply(ReplyOperation::InputItems, &broken).is_err());
}

#[test]
fn capabilities_never_fake_a_stream_over_a_websocket() {
    const {
        assert!(!SHOULD_FAKE_STREAM);
        assert!(!SUPPORTS_NATIVE_WEBSOCKET);
    }
}
