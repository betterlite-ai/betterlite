#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{HiddenParams, InitResponseObject, extract};
use crate::fields::payload::records::{AdditionalHeaders, StandardLoggingHiddenParams};

mod final_response;
mod ids;

fn decode(encoded: &str) -> Json {
    serde_json::from_str(encoded).unwrap()
}

fn object(encoded: &str) -> IndexMap<String, Json> {
    match decode(encoded) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object: {encoded}")
        }
    }
}

fn headers(encoded: &str) -> AdditionalHeaders {
    serde_json::from_str(encoded).unwrap()
}

#[rstest]
#[case::absent(InitResponseObject::Absent, None, "{}", None)]
#[case::mapping_is_logged_as_sent(
    InitResponseObject::Mapping(object(r#"{"id":"x","usage":{}}"#)),
    None,
    r#"{"id":"x","usage":{}}"#,
    None
)]
#[case::other_shape_is_empty(InitResponseObject::Other, None, "{}", None)]
#[case::projected_keeps_hidden_params(
    InitResponseObject::Projected {
        body: object(r#"{"id":"x"}"#),
        hidden_params: Some(object(r#"{"cache_key":"abc"}"#))
    },
    None,
    r#"{"id":"x"}"#,
    Some(r#"{"cache_key":"abc"}"#)
)]
#[case::projected_without_hidden_params(
    InitResponseObject::Projected {
        body: object(r#"{"id":"x"}"#),
        hidden_params: None
    },
    None,
    r#"{"id":"x"}"#,
    None
)]
#[case::exception_headers_build_the_empty_record(
    InitResponseObject::Absent,
    Some(r#"{"x_ratelimit_limit_requests":5,"llm_provider-x-request-id":"abc"}"#),
    "{}",
    Some(
        r#"{"model_id":null,"cache_key":null,"api_base":null,"response_cost":null,"litellm_overhead_time_ms":null,"additional_headers":{"x_ratelimit_limit_requests":5,"llm_provider-x-request-id":"abc"},"batch_models":null,"batch_successful_requests":null,"batch_failed_requests":null,"litellm_model_name":null,"usage_object":null}"#
    )
)]
fn logging_fields_behavior_tests_behavior(
    #[case] init: InitResponseObject,
    #[case] additional_headers: Option<&str>,
    #[case] expected_body: &str,
    #[case] expected_hidden: Option<&str>,
) {
    let extracted = extract(init, additional_headers.map(headers));
    assert_eq!(Json::Object(extracted.response_obj), decode(expected_body));
    match extracted.hidden_params {
        Some(HiddenParams::FromObject(fields)) => {
            assert_eq!(Json::Object(fields), decode(expected_hidden.unwrap()));
        }
        Some(HiddenParams::FromException(record)) => {
            let expected: StandardLoggingHiddenParams =
                serde_json::from_str(expected_hidden.unwrap()).unwrap();
            assert_eq!(*record, expected);
        }
        None => assert!(expected_hidden.is_none()),
    }
}
