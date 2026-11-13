use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::response_params::params::supported_openai_params;
use rstest::rstest;
use serde_json::Value;

use crate::api::responses_provider_contract::config::ResponsesProviderConfig as _;
use crate::api::responses_provider_contract::records::{
    CompactRequest, Headers, IdRequest, InputItemsOrder, InputItemsQuery, InputItemsRequest,
    Params, ResponsesLogging,
};
use crate::manus_responses::error::ManusResponsesError;

use super::support::{params, plain_config, response};

fn api_params(wire: &str) -> ResponsesApiRequestParams {
    serde_json::from_str(wire).expect("the fixture is a request params record")
}

fn mapped_params(wire: &str) -> Params {
    plain_config()
        .map_params(&api_params(wire), "manus-1.6", false)
        .expect("the params map")
}

#[test]
fn supported_params_match_the_openai_responses_set() {
    let supported = plain_config()
        .supported_params("manus-1.6")
        .expect("the set is static");
    assert_eq!(supported.len(), supported_openai_params().len());
    assert!(supported.contains(&String::from("input")));
    assert!(supported.contains(&String::from("extra_body")));
}

#[test]
fn map_params_drops_input_and_model_and_keeps_the_rest() {
    let mapped = mapped_params(r#"{"input":"hi","model":"manus-1.6","store":false}"#);
    assert!(!mapped.contains_key("input"));
    assert!(!mapped.contains_key("model"));
    assert_eq!(mapped.get("store"), Some(&Json::Bool(false)));
}

#[rstest]
#[case::below_the_minimum(8, 16)]
#[case::just_below(15, 16)]
#[case::at_the_minimum(16, 16)]
#[case::above_the_minimum(17, 17)]
fn map_params_clamps_max_output_tokens_up_to_the_minimum(
    #[case] requested: i64,
    #[case] expected: i64,
) {
    let mapped = mapped_params(&format!(
        r#"{{"input":"hi","model":"manus-1.6","max_output_tokens":{requested}}}"#
    ));
    assert_eq!(
        mapped.get("max_output_tokens"),
        Some(&Json::Number(expected.into()))
    );
}

#[test]
fn map_params_keeps_an_explicit_null_max_output_tokens() {
    let mapped = mapped_params(r#"{"input":"hi","model":"manus-1.6","max_output_tokens":null}"#);
    assert_eq!(mapped.get("max_output_tokens"), Some(&Json::Null));
}

fn event(wire: &str) -> Value {
    let chunk: Json = serde_json::from_str(wire).expect("the event is JSON");
    let streamed = plain_config()
        .transform_streaming_response("manus-1.6", &chunk, &mut ResponsesLogging::default())
        .expect("the event transforms");
    serde_json::to_value(streamed).expect("the event serializes")
}

#[test]
fn an_unknown_event_stays_a_generic_event() {
    assert_eq!(
        event(r#"{"type":"vendor.event","x":1}"#),
        serde_json::json!({"type": "vendor.event", "x": 1})
    );
}

#[test]
fn an_invalid_known_event_keeps_its_payload_and_type() {
    assert_eq!(
        event(r#"{"type":"response.created"}"#),
        serde_json::json!({"type": "response.created"})
    );
}

#[test]
fn a_chunk_without_a_type_is_refused() {
    let chunk: Json = serde_json::from_str(r#"{"x":1}"#).expect("the chunk is JSON");
    let error = plain_config()
        .transform_streaming_response("manus-1.6", &chunk, &mut ResponsesLogging::default())
        .expect_err("no type names no event");
    assert!(matches!(error, ManusResponsesError::StreamChunkWithoutType));
}

#[test]
fn a_non_object_chunk_is_refused() {
    let chunk: Json = serde_json::from_str(r#""hi""#).expect("the chunk is JSON");
    let error = plain_config()
        .transform_streaming_response("manus-1.6", &chunk, &mut ResponsesLogging::default())
        .expect_err("a string is not an event");
    assert!(matches!(error, ManusResponsesError::StreamChunkShape));
}

fn id_request(response_id: &str) -> IdRequest {
    IdRequest {
        response_id: response_id.to_owned(),
        api_base: String::from("https://api.manus.im/v1/responses"),
        litellm_params: Params::new(),
        headers: Headers::new(),
    }
}

#[test]
fn delete_and_cancel_requests_use_the_resource_paths() {
    let deleted = plain_config()
        .transform_delete_request(&id_request("a b"))
        .expect("delete maps");
    assert_eq!(deleted.url, "https://api.manus.im/v1/responses/a%20b");
    assert!(deleted.params.is_empty());
    let cancelled = plain_config()
        .transform_cancel_request(&id_request("resp_1"))
        .expect("cancel maps");
    assert_eq!(
        cancelled.url,
        "https://api.manus.im/v1/responses/resp_1/cancel"
    );
}

#[test]
fn input_items_requests_carry_the_default_query() {
    let request = InputItemsRequest {
        response_id: String::from("resp_1"),
        api_base: String::from("https://api.manus.im/v1/responses"),
        litellm_params: Params::new(),
        headers: Headers::new(),
        query: InputItemsQuery::default(),
    };
    let prepared = plain_config()
        .transform_input_items_request(&request)
        .expect("input items map");
    assert_eq!(
        prepared.url,
        "https://api.manus.im/v1/responses/resp_1/input_items"
    );
    assert_eq!(prepared.params.get("limit"), Some(&Json::Number(20.into())));
    assert_eq!(
        prepared.params.get("order"),
        Some(&Json::Text(String::from("desc")))
    );
}

#[test]
fn an_ascending_input_items_query_keeps_its_order() {
    let request = InputItemsRequest {
        response_id: String::from("resp_1"),
        api_base: String::from("https://api.manus.im/v1/responses"),
        litellm_params: Params::new(),
        headers: Headers::new(),
        query: InputItemsQuery {
            after: None,
            before: None,
            include: None,
            limit: 5,
            order: InputItemsOrder::Asc,
        },
    };
    let prepared = plain_config()
        .transform_input_items_request(&request)
        .expect("input items map");
    assert_eq!(
        prepared.params.get("order"),
        Some(&Json::Text(String::from("asc")))
    );
    assert_eq!(prepared.params.get("limit"), Some(&Json::Number(5.into())));
}

#[test]
fn compact_requests_reuse_the_base_builder() {
    let request = CompactRequest {
        model: String::from("manus/manus-1.6"),
        input: serde_json::from_str(r#""hi""#).expect("input is JSON"),
        optional_params: params(r#"{"store": false}"#),
        api_base: String::from("https://api.manus.im/v1/responses"),
        litellm_params: Params::new(),
        headers: Headers::new(),
    };
    let prepared = plain_config()
        .transform_compact_request(&request)
        .expect("compact maps");
    assert_eq!(prepared.url, "https://api.manus.im/v1/responses/compact");
    assert_eq!(
        prepared.params.get("model"),
        Some(&Json::Text(String::from("manus/manus-1.6")))
    );
    assert_eq!(
        prepared.params.get("input"),
        Some(&Json::Text(String::from("hi")))
    );
}

#[test]
fn delete_responses_decode_the_deleted_record() {
    let upstream = response(
        200,
        r#"{"id":"resp_1","object":"response.deleted","deleted":true}"#,
    );
    let deleted = plain_config()
        .transform_delete_response(&upstream, &mut ResponsesLogging::default())
        .expect("delete decodes");
    assert_eq!(deleted.id.as_deref(), Some("resp_1"));
    assert_eq!(deleted.deleted, Some(true));
}

#[test]
fn cancel_responses_are_typed_strictly() {
    let upstream = response(200, r#"{"id":"resp_1","created_at":9,"output":[]}"#);
    let reply = plain_config()
        .transform_cancel_response(&upstream, &mut ResponsesLogging::default())
        .expect("cancel decodes");
    assert_eq!(reply.id, "resp_1");
    assert_eq!(reply.created_at, 9);
}

#[test]
fn a_lax_cancel_response_is_refused() {
    let upstream = response(200, r#"{"id":7,"created_at":9,"output":"bad"}"#);
    let error = plain_config()
        .transform_cancel_response(&upstream, &mut ResponsesLogging::default())
        .expect_err("cancel validates strictly");
    assert!(matches!(error, ManusResponsesError::Resource(_)));
}

#[test]
fn input_items_responses_are_objects() {
    let upstream = response(200, r#"{"object":"list","data":[]}"#);
    let listed = plain_config()
        .transform_input_items_response(&upstream, &mut ResponsesLogging::default())
        .expect("the list decodes");
    assert_eq!(
        listed.get("object"),
        Some(&Json::Text(String::from("list")))
    );
    let not_object = response(200, "[]");
    let error = plain_config()
        .transform_input_items_response(&not_object, &mut ResponsesLogging::default())
        .expect_err("a list is not a params map");
    assert!(matches!(error, ManusResponsesError::InputItemsShape));
}
