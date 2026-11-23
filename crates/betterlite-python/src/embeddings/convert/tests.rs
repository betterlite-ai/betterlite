use super::{EmbeddingArguments, EmbeddingCallWire, NativeEmbeddingErrorPayload, parse};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use secrecy::ExposeSecret as _;
use serde_json::Number;
fn arguments(text: &str) -> EmbeddingArguments {
    let wire: EmbeddingCallWire = parse(text).unwrap();
    wire.arguments
}
fn context(message: &str) -> ErrorContext {
    ErrorContext {
        message: message.to_owned(),
        model: Some(String::from("text-embedding-3-small")),
        provider: Some(String::from("openai")),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}
#[test]
fn request_string_input_stays_a_string() {
    let call = arguments(r#"{"model": "text-embedding-3-small", "input": "hello"}"#);
    assert_eq!(call.model, "text-embedding-3-small");
    assert_eq!(
        call.input,
        Some(EmbeddingInput::Text(String::from("hello")))
    );
}
#[test]
fn request_text_list_input_stays_a_text_list() {
    let call = arguments(r#"{"model": "m", "input": ["a", "b"]}"#);
    assert_eq!(
        call.input,
        Some(EmbeddingInput::Texts(vec![
            String::from("a"),
            String::from("b")
        ]))
    );
}
#[test]
fn request_token_list_input_stays_tokens() {
    let call = arguments(r#"{"model": "m", "input": [1, 2, 3]}"#);
    assert_eq!(call.input, Some(EmbeddingInput::Tokens(vec![1_u32, 2, 3])));
}
#[test]
fn request_token_matrix_input_stays_token_lists() {
    let call = arguments(r#"{"model": "m", "input": [[1], [2, 3]]}"#);
    assert_eq!(
        call.input,
        Some(EmbeddingInput::TokenLists(vec![vec![1_u32], vec![2, 3]]))
    );
}
#[test]
fn request_missing_input_is_absent() {
    let call = arguments(r#"{"model": "m"}"#);
    assert_eq!(call.input, None);
}
#[test]
fn request_empty_input_stays_the_empty_list() {
    let call = arguments(r#"{"model": "m", "input": []}"#);
    assert_eq!(call.input, Some(EmbeddingInput::Texts(Vec::new())));
}
#[test]
fn request_unknown_kwargs_keep_the_shape_they_arrived_in() {
    let call = arguments(
        r#"{"model": "m", "input": "hi", "truncate_prompt_tokens": 4, "extra_body": {"z": 1}}"#,
    );
    assert_eq!(
        call.extra.get("truncate_prompt_tokens"),
        Some(&Json::Number(Number::from(4_u32)))
    );
    assert_eq!(
        call.extra.get("extra_body"),
        Some(&Json::Object(
            [(String::from("z"), Json::Number(Number::from(1_u32)))]
                .into_iter()
                .collect()
        ))
    );
}
#[test]
fn request_named_arguments_are_read_off_the_text() {
    let call = arguments(
        r#"{"model": "m", "input": "hi", "dimensions": 3, "encoding_format": "base64",
            "timeout": 12.5, "user": "alice", "api_base": "http://base", "api_key": "sk-key",
            "custom_llm_provider": "openai", "litellm_call_id": "call-1",
            "headers": {"x-base": "1"}, "mock_response": [0.5],
            "input_cost_per_token": 0.1, "output_cost_per_token": 0.2,
            "allowed_openai_params": ["dimensions"], "additional_drop_params": ["user"],
            "drop_params": true}"#,
    );
    assert_eq!(call.dimensions, Some(3));
    assert_eq!(call.encoding_format.as_deref(), Some("base64"));
    assert_eq!(call.timeout, Some(12.5_f64));
    assert_eq!(call.user.as_deref(), Some("alice"));
    assert_eq!(call.api_base.as_deref(), Some("http://base"));
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-key")
    );
    assert_eq!(call.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(call.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(
        call.headers
            .as_ref()
            .and_then(|headers| headers.get("x-base")),
        Some(&String::from("1"))
    );
    assert_eq!(
        call.mock_response,
        Some(Json::List(vec![Json::Number(
            Number::from_f64(0.5_f64).unwrap()
        )]))
    );
    assert_eq!(call.input_cost_per_token, Some(0.1_f64));
    assert_eq!(call.output_cost_per_token, Some(0.2_f64));
    assert_eq!(
        call.allowed_openai_params,
        Some(vec![String::from("dimensions")])
    );
    assert_eq!(
        call.additional_drop_params,
        Some(vec![String::from("user")])
    );
    assert_eq!(call.drop_params, Some(true));
}
#[test]
fn request_settings_are_read_off_the_text() {
    let wire: EmbeddingCallWire = parse(
        r#"{"arguments": {"model": "m"},
            "settings": {"api_base": "http://module", "api_key": "sk-module",
                         "openai_key": "sk-openai", "drop_params": true}}"#,
    )
    .unwrap();
    assert_eq!(wire.settings.api_base.as_deref(), Some("http://module"));
    assert_eq!(
        wire.settings
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-module")
    );
    assert_eq!(
        wire.settings
            .openai_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-openai")
    );
    assert_eq!(wire.settings.drop_params, Some(true));
}
#[test]
fn request_a_malformed_text_is_a_conversion_error() {
    parse::<EmbeddingCallWire>("not json").unwrap_err();
}
#[test]
fn error_payload_keeps_the_authentication_class() {
    let class = ErrorClass::Authentication(Box::new(ResponseDetails::<401>::from(context(
        "AuthenticationError: OpenAIException - bad key",
    ))));
    let payload = NativeEmbeddingErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401));
    assert_eq!(payload.model.as_deref(), Some("text-embedding-3-small"));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        payload.message,
        "AuthenticationError: OpenAIException - bad key"
    );
}
#[test]
fn error_payload_keeps_a_plain_exception_without_a_status() {
    let class = ErrorClass::Unclassified(Box::new(ErrorContext {
        message: String::from("Mock error"),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }));
    let payload = NativeEmbeddingErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.status_code, None);
    assert_eq!(payload.message, "Mock error");
}
#[test]
fn error_payload_keeps_unsupported_params_at_400() {
    let class = ErrorClass::UnsupportedParams(Box::new(ErrorContext {
        message: String::from("Setting dimensions is not supported"),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }));
    let payload = NativeEmbeddingErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "UnsupportedParamsError");
    assert_eq!(payload.status_code, Some(400));
    assert!(payload.model.is_none());
    assert!(payload.headers.is_empty());
}
#[test]
fn error_payload_serializes_every_key_b5659_carries() {
    let class = ErrorClass::BadRequest(Box::new(ResponseDetails::<400>::from(context(
        "bad request",
    ))));
    let payload = NativeEmbeddingErrorPayload::from_class(&class);
    let text = serde_json::to_string(&payload).unwrap();
    let value: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = value else {
        panic!("the error payload is not an object");
    };
    for name in [
        "type",
        "message",
        "status_code",
        "llm_provider",
        "model",
        "body",
        "headers",
        "num_retries",
        "max_retries",
    ] {
        assert!(fields.contains_key(name), "{name} is missing");
    }
}
