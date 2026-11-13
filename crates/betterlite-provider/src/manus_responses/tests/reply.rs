use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;

use crate::api::responses_provider_contract::config::ResponsesProviderConfig as _;
use crate::api::responses_provider_contract::error::{BaseLlmException, ResponsesContractError};
use crate::api::responses_provider_contract::records::{
    Headers, Params, ResponsesLogging, ResponsesUpstream,
};
use crate::manus_responses::error::ManusResponsesError;
use crate::manus_responses::{UnknownIdSource, UuidIdSource};

use super::support::{SUFFIX, headered_response, plain_config, response};

fn created(upstream: &ResponsesUpstream) -> (Params, ResponsesLogging) {
    let mut logging = ResponsesLogging::default();
    let reply = plain_config()
        .transform_create_response("manus/manus-1.6", upstream, &mut logging)
        .expect("the fixture reply transforms");
    (indexmap_to_params(&reply), logging)
}

fn indexmap_to_params(response: &ResponsesApiResponse) -> Params {
    serde_json::from_value(serde_json::to_value(response).expect("the reply serializes"))
        .expect("the reply is a JSON object")
}

#[test]
fn created_at_comes_from_camel_case_created_at() {
    let upstream = response(
        200,
        r#"{"id":"resp_1","createdAt":1234567890.5,"output":[]}"#,
    );
    let (reply, _logging) = created(&upstream);
    assert_eq!(
        reply.get("created_at"),
        Some(&Json::Number(1_234_567_890_i64.into()))
    );
}

#[test]
fn a_present_created_at_wins_and_is_still_converted() {
    let upstream = response(
        200,
        r#"{"id":"resp_1","created_at":"7.9","createdAt":5,"output":[]}"#,
    );
    let (reply, _logging) = created(&upstream);
    assert_eq!(reply.get("created_at"), Some(&Json::Number(7.into())));
}

#[test]
fn missing_fields_get_the_manus_defaults() {
    let upstream = response(200, r#"{"id":"resp_1","created_at":5}"#);
    let (reply, _logging) = created(&upstream);
    assert_eq!(reply.get("reasoning"), Some(&Json::Object(IndexMap::new())));
    assert_eq!(reply.get("text"), Some(&Json::Object(IndexMap::new())));
    assert_eq!(reply.get("output"), Some(&Json::List(Vec::new())));
    let usage = reply
        .get("usage")
        .and_then(Json::as_object)
        .expect("the default usage stays an object");
    assert_eq!(usage.get("input_tokens"), Some(&Json::Number(0.into())));
    assert_eq!(usage.get("output_tokens"), Some(&Json::Number(0.into())));
    assert_eq!(usage.get("total_tokens"), Some(&Json::Number(0.into())));
}

#[test]
fn null_fields_get_the_manus_defaults() {
    let upstream = response(
        200,
        r#"{"id":null,"created_at":5,"reasoning":null,"text":null,"output":null,"usage":null}"#,
    );
    let (reply, _logging) = created(&upstream);
    assert_eq!(
        reply.get("id"),
        Some(&Json::Text(format!("unknown-{SUFFIX}")))
    );
    assert_eq!(reply.get("reasoning"), Some(&Json::Object(IndexMap::new())));
    assert_eq!(reply.get("text"), Some(&Json::Object(IndexMap::new())));
    assert_eq!(reply.get("output"), Some(&Json::List(Vec::new())));
    assert!(matches!(reply.get("usage"), Some(Json::Object(_))));
}

#[test]
fn the_same_reply_routine_answers_create_and_get() {
    let upstream = response(200, r#"{"id":"resp_1","createdAt":5,"output":[]}"#);
    let mut create_logging = ResponsesLogging::default();
    let mut get_logging = ResponsesLogging::default();
    let created = plain_config()
        .transform_create_response("manus/manus-1.6", &upstream, &mut create_logging)
        .expect("create transforms");
    let retrieved = plain_config()
        .transform_get_response(&upstream, &mut get_logging)
        .expect("get transforms");
    assert_eq!(created, retrieved);
    assert_eq!(
        create_logging.model_call_details,
        get_logging.model_call_details
    );
}

#[test]
fn invalid_json_raises_the_openai_error_with_the_status_and_text() {
    let upstream = response(503, "not json");
    let error = plain_config()
        .transform_create_response(
            "manus/manus-1.6",
            &upstream,
            &mut ResponsesLogging::default(),
        )
        .expect_err("invalid JSON is refused");
    let ManusResponsesError::Contract(ResponsesContractError::BaseLlm(BaseLlmException {
        status_code,
        message,
        headers,
    })) = error
    else {
        panic!("the failure should be the base exception");
    };
    assert_eq!(status_code, 503);
    assert_eq!(message, "not json");
    assert_eq!(headers, Headers::new());
}

#[test]
fn a_non_object_body_raises_the_same_error() {
    let upstream = response(400, "[]");
    let error = plain_config()
        .transform_get_response(&upstream, &mut ResponsesLogging::default())
        .expect_err("a list is not a reply record");
    let ManusResponsesError::Contract(ResponsesContractError::BaseLlm(inner)) = error else {
        panic!("the failure should be the base exception");
    };
    assert_eq!(inner.status_code, 400);
    assert_eq!(inner.message, "[]");
}

#[test]
fn a_body_that_fails_validation_is_still_returned() {
    let upstream = response(200, r#"{"id":7,"created_at":5,"output":"bad"}"#);
    let mut logging = ResponsesLogging::default();
    let reply = crate::manus_responses::reply::transform(&plain_config(), &upstream, &mut logging)
        .expect("a mistyped body is still returned");
    assert_eq!(reply.id, format!("unknown-{SUFFIX}"));
    assert_eq!(reply.created_at, 5);
    assert!(reply.output.is_empty());
}

#[test]
fn post_call_records_the_raw_text_first() {
    let upstream = response(200, r#"{"id":"resp_1","created_at":5,"output":[]}"#);
    let (_, logging) = created(&upstream);
    assert_eq!(
        logging.model_call_details.get("original_response"),
        Some(&Json::Text(String::from(
            r#"{"id":"resp_1","created_at":5,"output":[]}"#
        )))
    );
    assert_eq!(
        logging.model_call_details.get("additional_args"),
        Some(&Json::Object(IndexMap::from([(
            String::from("complete_input_dict"),
            Json::Object(IndexMap::new())
        )])))
    );
    assert_eq!(
        logging.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
}

#[test]
fn the_processed_and_raw_headers_are_recorded() {
    let upstream = headered_response(
        200,
        r#"{"id":"resp_1","created_at":5,"output":[]}"#,
        Headers::from([
            (String::from("x-request-id"), String::from("req-1")),
            (
                String::from("x-ratelimit-remaining-requests"),
                String::from("9"),
            ),
            (String::from("llm_provider-x-custom"), String::from("keep")),
        ]),
    );
    let (_, logging) = created(&upstream);
    assert_eq!(
        logging.model_call_details.get("headers"),
        Some(&Json::Object(IndexMap::from([
            (
                String::from("x-request-id"),
                Json::Text(String::from("req-1"))
            ),
            (
                String::from("x-ratelimit-remaining-requests"),
                Json::Text(String::from("9"))
            ),
            (
                String::from("llm_provider-x-custom"),
                Json::Text(String::from("keep"))
            ),
        ])))
    );
    let Some(Json::Object(additional)) = logging.model_call_details.get("additional_headers")
    else {
        panic!("the processed headers should be a JSON object");
    };
    assert_eq!(
        additional.get("x-ratelimit-remaining-requests"),
        Some(&Json::Text(String::from("9")))
    );
    assert_eq!(
        additional.get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("req-1")))
    );
    assert_eq!(
        additional.get("llm_provider-x-custom"),
        Some(&Json::Text(String::from("keep")))
    );
}

#[test]
fn a_usage_object_is_kept_as_sent() {
    let upstream = response(
        200,
        r#"{"id":"resp_1","created_at":5,"output":[],"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}}"#,
    );
    let (reply, _logging) = created(&upstream);
    let usage = reply
        .get("usage")
        .and_then(Json::as_object)
        .expect("the usage stays an object");
    assert_eq!(usage.get("input_tokens"), Some(&Json::Number(3.into())));
    assert_eq!(usage.get("output_tokens"), Some(&Json::Number(4.into())));
}

#[test]
fn the_default_id_source_yields_eight_hex_characters() {
    let suffix = UuidIdSource.next_suffix();
    assert_eq!(suffix.len(), 8);
    assert!(
        suffix
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
}

#[test]
fn the_unknown_id_keeps_the_configured_suffix() {
    let upstream = response(200, r#"{"created_at":5,"output":[]}"#);
    let (reply, _logging) = created(&upstream);
    assert_eq!(
        reply.get("id"),
        Some(&Json::Text(format!("unknown-{SUFFIX}")))
    );
}
