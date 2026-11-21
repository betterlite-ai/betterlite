#![cfg(test)]

use super::{
    CallWire, ConvertError, NativeErrorPayload, OutcomeParts, ProbeCall, SessionWire, TokenCall,
    outcome_parts, parse,
};
use betterlite_error::error::details::{
    RateLimitDetails, ResponseDetails, SchemaValidationDetails,
};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;
use betterlite_sdk::call_setup::correlation::{CallOutcome, ChatStream};
use betterlite_sdk::call_wrapper::error::{CallError, CallErrorKind, plain_error};
use std::collections::BTreeMap;

fn context(message: &str, model: Option<&str>, provider: Option<&str>) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: model.map(str::to_owned),
        provider: provider.map(str::to_owned),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}

#[test]
fn request_text_carries_arguments_and_the_settings_snapshot() {
    let text = r#"{
        "arguments": {"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}], "temperature": 0.5},
        "settings": {
            "drop_params": true,
            "request_timeout": 12.5,
            "model_alias_map": {"fast": "gpt-4o-mini"},
            "proxy_headers": {"X-Title": "tests"},
            "optional": {"vertex_project": "project"},
            "add_function_to_prompt": true,
            "disable_stop_sequence_limit": true
        }
    }"#;
    let wire = parse::<CallWire>(text).unwrap();
    assert_eq!(
        wire.arguments.get("model"),
        Some(&Json::Text(String::from("gpt-4o")))
    );
    assert_eq!(
        wire.arguments.get("messages"),
        Some(&Json::List(vec![Json::Object(
            [
                (String::from("role"), Json::Text(String::from("user"))),
                (String::from("content"), Json::Text(String::from("hi"))),
            ]
            .into_iter()
            .collect(),
        ),]))
    );
    let settings = wire.settings.into_settings(2048_u32);
    assert_eq!(settings.drop_params, Some(true));
    assert_eq!(
        settings.request_timeout.map(f64::to_bits),
        Some(12.5_f64.to_bits())
    );
    assert_eq!(
        settings.model_alias_map.get("fast"),
        Some(&String::from("gpt-4o-mini"))
    );
    assert_eq!(
        settings.proxy_headers.get("X-Title"),
        Some(&Json::Text(String::from("tests")))
    );
    assert_eq!(settings.optional.vertex_project.as_deref(), Some("project"));
    assert!(settings.flags.add_function_to_prompt);
    assert!(settings.flags.disable_stop_sequence_limit);
    assert_eq!(settings.medium_thinking_budget, 2048_u32);
}

#[test]
fn request_absent_fields_take_litellm_defaults() {
    let wire = parse::<CallWire>("{}").unwrap();
    assert!(wire.arguments.is_empty());
    let settings = wire.settings.into_settings(2048_u32);
    assert_eq!(settings.drop_params, None);
    assert_eq!(settings.num_retries, None);
    assert_eq!(settings.request_timeout, None);
    assert!(settings.api_key.is_none());
    assert_eq!(settings.api_base, None);
    assert!(!settings.route_all_to_responses);
    assert!(settings.proxy_headers.is_empty());
    assert!(!settings.flags.add_function_to_prompt);
    assert!(!settings.flags.disable_stop_sequence_limit);
    assert_eq!(settings.medium_thinking_budget, 2048_u32);
}

#[test]
fn request_settings_override_the_medium_thinking_budget() {
    let wire = parse::<CallWire>(r#"{"settings": {"medium_thinking_budget": 4096}}"#).unwrap();
    assert_eq!(
        wire.settings.into_settings(2048_u32).medium_thinking_budget,
        4096_u32
    );
}

#[test]
fn request_malformed_text_is_a_json_error() {
    let error = parse::<CallWire>("not json").unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
}

#[test]
fn request_wrong_shape_is_a_json_error() {
    let error = parse::<CallWire>(r#"{"arguments": 3}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
    let error = parse::<TokenCall>(r#"{"model": 3}"#).unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
    let error = parse::<SessionWire>("[]").unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
}

#[test]
fn request_token_call_keeps_model_messages_and_system() {
    let text = r#"{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
        "tools": [{"type": "function", "function": {"name": "f"}}],
        "system": "be brief",
        "api_key": "sk-test",
        "api_base": "http://127.0.0.1:1"
    }"#;
    let call = parse::<TokenCall>(text).unwrap();
    assert_eq!(call.model, "gpt-4o");
    assert_eq!(call.messages.unwrap().len(), 1);
    assert_eq!(call.tools.unwrap().len(), 1);
    assert_eq!(call.system.as_deref(), Some("be brief"));
    assert!(call.api_key.is_some());
    assert_eq!(call.api_base.as_deref(), Some("http://127.0.0.1:1"));
}

#[test]
fn request_token_call_without_messages_counts_nothing() {
    let call = parse::<TokenCall>(r#"{"model": "gpt-4o"}"#).unwrap();
    assert_eq!(call.model, "gpt-4o");
    assert_eq!(call.messages, None);
    assert_eq!(call.tools, None);
    assert_eq!(call.system, None);
    assert!(call.api_key.is_none());
    assert_eq!(call.api_base, None);
}

#[test]
fn request_probe_call_keeps_model_and_key() {
    let call = parse::<ProbeCall>(r#"{"model": "gpt-4o", "api_key": "sk-test"}"#).unwrap();
    assert_eq!(call.model, "gpt-4o");
    assert!(call.api_key.is_some());
    let call = parse::<ProbeCall>(r#"{"model": "gpt-4o"}"#).unwrap();
    assert_eq!(call.model, "gpt-4o");
    assert!(call.api_key.is_none());
}

#[test]
fn request_probe_call_becomes_the_model_and_key_arguments() {
    let call = parse::<ProbeCall>(r#"{"model": "gpt-4o", "api_key": "sk-test"}"#).unwrap();
    let arguments = call.arguments();
    assert_eq!(arguments.len(), 2);
    assert_eq!(
        arguments.get("model"),
        Some(&Json::Text(String::from("gpt-4o")))
    );
    assert_eq!(
        arguments.get("api_key"),
        Some(&Json::Text(String::from("sk-test")))
    );
    let keyless = parse::<ProbeCall>(r#"{"model": "gpt-4o"}"#).unwrap();
    assert_eq!(keyless.arguments().get("api_key"), None);
    assert_eq!(keyless.arguments().len(), 1);
}

#[test]
fn request_null_flags_take_their_defaults() {
    let text = r#"{"settings": {"route_all_to_responses": null, "add_function_to_prompt": null, "disable_stop_sequence_limit": null}}"#;
    let settings = parse::<CallWire>(text)
        .unwrap()
        .settings
        .into_settings(2048_u32);
    assert!(!settings.route_all_to_responses);
    assert!(!settings.flags.add_function_to_prompt);
    assert!(!settings.flags.disable_stop_sequence_limit);
    let routed = parse::<CallWire>(r#"{"settings": {"route_all_to_responses": true}}"#)
        .unwrap()
        .settings
        .into_settings(2048_u32);
    assert!(routed.route_all_to_responses);
}

#[test]
fn request_session_reads_the_environment_snapshot() {
    let wire = parse::<SessionWire>(
        r#"{"environment": {"OPENAI_API_KEY": "sk-test"}, "medium_thinking_budget": 1024}"#,
    )
    .unwrap();
    assert_eq!(
        wire.environment,
        Some(BTreeMap::from([(
            String::from("OPENAI_API_KEY"),
            String::from("sk-test")
        )]))
    );
    assert_eq!(wire.medium_thinking_budget, Some(1024_u32));
}

#[test]
fn request_session_without_environment_leaves_it_to_the_process() {
    let wire = parse::<SessionWire>("{}").unwrap();
    assert_eq!(wire.environment, None);
    assert_eq!(wire.medium_thinking_budget, None);
}

#[test]
fn error_unclassified_is_a_plain_exception() {
    let error = CallError::from(plain_error("boom"));
    let payload = NativeErrorPayload::from_call_error(&error);
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.message, "boom");
    assert_eq!(payload.status_code, None);
    assert_eq!(payload.llm_provider, None);
    assert_eq!(payload.model, None);
    assert_eq!(payload.body, None);
    assert!(payload.headers.is_empty());
    assert_eq!(payload.num_retries, None);
    assert_eq!(payload.max_retries, None);
    let text = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        text,
        r#"{"type":"Exception","message":"boom","status_code":null,"llm_provider":null,"model":null,"body":null,"headers":{},"num_retries":null,"max_retries":null}"#
    );
}

#[test]
fn error_rate_limit_carries_the_status_the_headers_and_the_retry_counters() {
    let details = RateLimitDetails {
        context: ErrorContext {
            max_retries: Some(2_u32),
            num_retries: Some(1_u32),
            ..context("rate limited", Some("gpt-4o"), Some("openai"))
        },
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::from([(String::from("retry-after"), String::from("5"))]),
    };
    let error = CallError {
        class: ErrorClass::RateLimit(Box::new(details)),
        kind: CallErrorKind::Normal,
        num_retries: Some(3_i64),
        timeout: None,
    };
    let payload = NativeErrorPayload::from_call_error(&error);
    assert_eq!(payload.kind, "RateLimitError");
    assert_eq!(payload.message, "rate limited");
    assert_eq!(payload.status_code, Some(429_u16));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
    assert_eq!(payload.model.as_deref(), Some("gpt-4o"));
    assert_eq!(
        payload.headers.get("retry-after").map(String::as_str),
        Some("5")
    );
    assert_eq!(payload.num_retries, Some(3_i64));
    assert_eq!(payload.max_retries, Some(2_i64));
    let text = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        text,
        r#"{"type":"RateLimitError","message":"rate limited","status_code":429,"llm_provider":"openai","model":"gpt-4o","body":null,"headers":{"retry-after":"5"},"num_retries":3,"max_retries":2}"#
    );
}

#[test]
fn error_authentication_carries_its_default_status_and_no_provider_headers() {
    let details = ResponseDetails::<401>::from(context("bad key", Some("gpt-4o"), Some("openai")));
    let error = CallError::from(ErrorClass::Authentication(Box::new(details)));
    let payload = NativeErrorPayload::from_call_error(&error);
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401_u16));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
    assert!(payload.headers.is_empty());
    assert_eq!(payload.num_retries, None);
    assert_eq!(payload.max_retries, None);
}

#[test]
fn error_schema_validation_is_not_a_plain_api_error() {
    let error = CallError::from(ErrorClass::JsonSchemaValidation(Box::new(
        SchemaValidationDetails {
            model: String::from("gpt-4o"),
            provider: String::from("openai"),
            raw_response: String::from("{}"),
            schema: String::from("{}"),
        },
    )));
    let payload = NativeErrorPayload::from_call_error(&error);
    assert_eq!(payload.kind, "JSONSchemaValidationError");
}

#[test]
fn response_outcome_becomes_json_text() {
    let parts = outcome_parts(CallOutcome::Response(Json::Text(String::from("done")))).unwrap();
    assert_eq!(parts, OutcomeParts::Response(String::from("\"done\"")));
}

#[test]
fn stream_outcome_yields_one_text_per_buffered_chunk() {
    let stream = ChatStream {
        model_response: Json::List(vec![
            Json::Text(String::from("a")),
            Json::Text(String::from("b")),
        ]),
        json_mode: false,
        converted_from_mock: false,
    };
    let parts = outcome_parts(CallOutcome::ChatStream(stream)).unwrap();
    assert_eq!(
        parts,
        OutcomeParts::Stream(vec![String::from("\"a\""), String::from("\"b\"")])
    );
}

#[test]
fn responses_stream_outcome_with_one_payload_is_one_chunk() {
    let parts = outcome_parts(CallOutcome::ResponsesStream(Json::Text(String::from("x")))).unwrap();
    assert_eq!(parts, OutcomeParts::Stream(vec![String::from("\"x\"")]));
}
