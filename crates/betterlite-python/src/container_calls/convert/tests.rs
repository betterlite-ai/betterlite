#![cfg(test)]

use super::{
    CallWire, ConvertError, CreatedAnswer, NativeContainerPayload, parse, query_values,
    requested_timeout,
};
use betterlite_error::error::details::{RateLimitDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::ContainerObject;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use indexmap::IndexMap;
use serde_json::Number;
use std::collections::BTreeMap;
use std::time::Duration;

fn context() -> ErrorContext {
    ErrorContext {
        message: String::from("no such container"),
        model: Some(String::from("")),
        provider: Some(String::from("openai")),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

fn wire(text: &str) -> CallWire {
    parse(text).unwrap()
}

fn container() -> ContainerObject {
    serde_json::from_str(
        r#"{"id":"cntr_1","object":"container","created_at":1,"status":"running",
            "expires_after":null,"last_active_at":null,"name":"box"}"#,
    )
    .unwrap()
}

#[test]
fn request_create_keeps_its_named_arguments() {
    let call = wire(
        r#"{"name":"box","expires_after":{"anchor":"last_active_at","minutes":20},
            "file_ids":["file_1"]}"#,
    )
    .arguments
    .create()
    .unwrap();
    assert_eq!(call.name, "box");
    assert_eq!(
        call.expires_after
            .as_ref()
            .and_then(|after| after.get("minutes")),
        Some(&serde_json::json!(20))
    );
    assert_eq!(call.file_ids, Some(Vec::from([String::from("file_1")])));
}

#[test]
fn request_create_keeps_the_connection_arguments() {
    let call = wire(
        r#"{"name":"box","timeout":30.0,"api_key":"sk-test",
            "api_base":"https://example.test/v1","api_version":"v1",
            "custom_llm_provider":"openai","extra_headers":{"X-Test":"1"}}"#,
    )
    .arguments
    .create()
    .unwrap();
    assert_eq!(call.timeout, Some(30.0_f64));
    assert!(call.api_key.is_some());
    assert_eq!(call.api_base.as_deref(), Some("https://example.test/v1"));
    assert_eq!(call.api_version.as_deref(), Some("v1"));
    assert_eq!(call.custom_llm_provider, "openai");
    assert_eq!(
        call.extra_headers
            .as_ref()
            .and_then(|headers| headers.get("X-Test")),
        Some(&String::from("1"))
    );
}

#[test]
fn request_create_keeps_the_passthrough_maps() {
    let call = wire(
        r#"{"name":"box","extra_query":{"after":"cntr_0"},"extra_body":{"trace":"1"},
            "litellm_call_id":"call_1","litellm_metadata":{"model_info":{"id":"m1"}},
            "ssl_verify":false}"#,
    )
    .arguments
    .create()
    .unwrap();
    assert_eq!(
        call.extra_query
            .as_ref()
            .and_then(|query| query.get("after")),
        Some(&ParamValue::Text(String::from("cntr_0")))
    );
    assert_eq!(
        call.extra_body.as_ref().and_then(|body| body.get("trace")),
        Some(&String::from("1"))
    );
    assert_eq!(call.litellm_call_id.as_deref(), Some("call_1"));
    assert_eq!(
        call.litellm_metadata
            .as_ref()
            .and_then(|metadata| metadata.get("model_info")),
        Some(&JsonValue::Object(IndexMap::from([(
            String::from("id"),
            JsonValue::Text(String::from("m1"))
        )])))
    );
    assert_eq!(call.extra.get("ssl_verify"), Some(&Json::Bool(false)));
    assert!(!call.extra.contains_key("name"));
}

#[test]
fn request_create_defaults_the_provider_to_openai() {
    let call = wire(r#"{"name":"box"}"#).arguments.create().unwrap();
    assert_eq!(call.custom_llm_provider, "openai");
}

#[test]
fn request_create_passes_a_zero_and_an_absent_timeout_unresolved() {
    let absent = wire(r#"{"name":"box"}"#).arguments.create().unwrap();
    assert_eq!(absent.timeout, None);
    let zero = wire(r#"{"name":"box","timeout":0}"#)
        .arguments
        .create()
        .unwrap();
    assert_eq!(zero.timeout, Some(0.0_f64));
}

#[test]
fn request_list_leaves_an_unset_limit_and_order_absent() {
    let call = wire(r#"{"after":"cntr_0"}"#).arguments.list().unwrap();
    assert_eq!(call.after.as_deref(), Some("cntr_0"));
    assert_eq!(call.limit, None);
    assert_eq!(call.order, None);
}

#[test]
fn request_list_keeps_a_limit_of_zero_and_a_given_order() {
    let call = wire(r#"{"limit":0,"order":"asc"}"#)
        .arguments
        .list()
        .unwrap();
    assert_eq!(call.limit, Some(0_i64));
    assert_eq!(call.order.as_deref(), Some("asc"));
}

#[test]
fn request_extra_query_keeps_text_and_integers() {
    let call = wire(r#"{"extra_query":{"after":"cntr_0","limit":5}}"#)
        .arguments
        .list()
        .unwrap();
    let query = call.extra_query.unwrap();
    assert_eq!(
        query.get("after"),
        Some(&ParamValue::Text(String::from("cntr_0")))
    );
    assert_eq!(query.get("limit"), Some(&ParamValue::Integer(5)));
}

#[test]
fn request_extra_query_names_a_value_that_cannot_be_sent() {
    let values = IndexMap::from([(String::from("limit"), Json::Bool(true))]);
    let error = query_values(Some(&values)).unwrap_err();
    assert!(matches!(error, ConvertError::QueryValue { ref name } if name == "limit"));
}

#[test]
fn request_extra_query_answers_none_for_an_absent_map() {
    let converted = query_values(None).unwrap();
    assert_eq!(converted, None);
}

#[test]
fn request_operation_carries_every_name_it_reads() {
    let call = wire(
        r#"{"container_id":"cntr_1","timeout":30.0,"api_key":"sk-test",
            "custom_llm_provider":"openai","extra_headers":{"X-Test":"1"},
            "extra_query":{"after":"cntr_0"},"litellm_call_id":"call_1",
            "litellm_metadata":{"model_info":{"id":"m1"}},"ssl_verify":false}"#,
    );
    let extra_query = query_values(call.arguments.extra_query.as_ref()).unwrap();
    let request = call
        .arguments
        .operation_request(extra_query.as_ref(), Some(Duration::from_secs(30)));
    assert_eq!(request.container_id, "cntr_1");
    assert_eq!(request.timeout, Some(Duration::from_secs(30)));
    assert_eq!(request.custom_llm_provider, Some("openai"));
    assert_eq!(request.litellm_call_id, Some("call_1"));
    assert_eq!(
        request.extra_query.and_then(|query| query.get("after")),
        Some(&ParamValue::Text(String::from("cntr_0")))
    );
    assert_eq!(
        request.extras.and_then(|extras| extras.get("ssl_verify")),
        Some(&Json::Bool(false))
    );
    assert_eq!(request.extra_body, None);
}

#[test]
fn request_timeout_leaves_an_absent_timeout_for_the_operation_to_resolve() {
    let timeout = requested_timeout(None).unwrap();
    assert_eq!(timeout, None);
}

#[test]
fn request_timeout_keeps_a_given_number_of_seconds() {
    let timeout = requested_timeout(Some(600.0_f64)).unwrap();
    assert_eq!(timeout, Some(Duration::from_secs(600)));
}

#[test]
fn request_timeout_keeps_zero_for_the_operation_to_resolve() {
    let timeout = requested_timeout(Some(0.0_f64)).unwrap();
    assert_eq!(timeout, Some(Duration::ZERO));
}

#[test]
fn error_timeout_refuses_a_negative_number() {
    let error = requested_timeout(Some(-1.0_f64)).unwrap_err();
    let ConvertError::Timeout { seconds, .. } = error else {
        panic!("a negative timeout is not a usable duration");
    };
    assert_eq!(seconds.to_bits(), (-1.0_f64).to_bits());
}

#[test]
fn request_create_answer_keeps_the_hidden_params() {
    let mut created = container();
    created.hidden_params.insert(
        String::from("additional_headers"),
        serde_json::json!({"llm_provider-x-litellm-response-cost": 0.03_f64}),
    );
    let text = serde_json::to_string(&CreatedAnswer {
        container: &created,
        hidden_params: &created.hidden_params,
    })
    .unwrap();
    let answer: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = answer else {
        panic!("the answer is a JSON object");
    };
    assert_eq!(fields.get("id"), Some(&Json::Text(String::from("cntr_1"))));
    let Some(Json::Object(hidden)) = fields.get("_hidden_params") else {
        panic!("the answer carries the hidden params");
    };
    let Some(Json::Object(headers)) = hidden.get("additional_headers") else {
        panic!("the hidden params carry the additional headers");
    };
    assert_eq!(
        headers.get("llm_provider-x-litellm-response-cost"),
        Some(&Json::Number(Number::from_f64(0.03_f64).unwrap()))
    );
}

#[test]
fn error_payload_keeps_the_class_message_and_status() {
    let class = ErrorClass::NotFound(Box::new(ResponseDetails::<404>::from(context())));
    let payload = NativeContainerPayload::from_class(&class);
    assert_eq!(payload.kind, "NotFoundError");
    assert_eq!(payload.message, "no such container");
    assert_eq!(payload.status_code, Some(404));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
    assert_eq!(payload.model.as_deref(), Some(""));
    assert!(payload.headers.is_empty());
    assert_eq!(payload.body, None);
}

#[test]
fn error_payload_of_a_plain_exception_has_no_status() {
    let class = ErrorClass::Unclassified(Box::new(context()));
    let payload = NativeContainerPayload::from_class(&class);
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.status_code, None);
}

#[test]
fn error_payload_carries_the_rate_limit_headers() {
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: context(),
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::from([(String::from("retry-after"), String::from("1"))]),
    }));
    let payload = NativeContainerPayload::from_class(&class);
    assert_eq!(payload.kind, "RateLimitError");
    assert_eq!(payload.status_code, Some(429));
    assert_eq!(
        payload.headers.get("retry-after").map(String::as_str),
        Some("1")
    );
}

#[test]
fn error_parse_refuses_a_text_that_is_not_json() {
    let error = parse::<CallWire>("not json").unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
}

#[test]
fn error_parse_refuses_a_declared_keyword_of_the_wrong_type() {
    let error = parse::<CallWire>(r#"{"container_id":5}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
}
