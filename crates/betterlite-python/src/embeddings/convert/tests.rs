use super::super::call::{Entry, call};
use super::super::error::{CallFacts, CallFailure, Raised, operation_failure, raised};
use super::super::request::build;
use super::super::response::{
    EmbeddingAnswer, HiddenFacts, hidden_params, mock_usage, record_json, response_cost,
};
use super::super::session::{
    EmbeddingSession, additional_headers, header_map, mock_api_base, openai_defaults, secret_text,
};
use super::{
    EmbeddingArguments, EmbeddingCallWire, EmbeddingSettingsWire, NativeEmbeddingErrorPayload,
};
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::embedding::{
    EmbeddingInput, EmbeddingRecord, EmbeddingRequest, EmbeddingVector, EncodingFormat,
};
use betterlite_provider::operation::OperationError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::{HeaderMap, HeaderValue};
use pyo3_async_runtimes::tokio::get_runtime;
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde_json::Number;
use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
const SMALL: &str = "text-embedding-3-small";
const ADA: &str = "text-embedding-ada-002";
const EMBEDDING_BODY: &str = r#"{"object": "list", "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}], "model": "text-embedding-3-small", "usage": {"prompt_tokens": 2, "total_tokens": 2}}"#;
const UNAUTHORIZED_BODY: &str = r#"{"error": {"message": "Invalid API key provided: bad-key", "type": "invalid_request_error", "param": null, "code": "invalid_api_key"}}"#;
fn parse<T: DeserializeOwned>(text: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(text)
}
fn session() -> EmbeddingSession {
    EmbeddingSession::build(r#"{"environment": {}}"#).unwrap()
}
fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
fn built(text: &str, model: &str, pairs: &[(&str, &str)]) -> Result<EmbeddingRequest, CallFailure> {
    build(
        &arguments(text),
        model,
        "openai",
        false,
        &environment(pairs),
    )
}
fn payload_of(failure: CallFailure) -> NativeEmbeddingErrorPayload {
    let CallFailure::Class(class) = failure else {
        panic!("the failure is not a LiteLLM exception class");
    };
    NativeEmbeddingErrorPayload::from_class(&class)
}
fn text_of(value: &Json) -> String {
    serde_json::to_string(value).unwrap()
}
fn priced(price: Decimal, tokens: u32) -> Json {
    let cost = price.checked_mul(Decimal::from(tokens)).unwrap();
    Json::Number(Number::from_f64(cost.to_f64().unwrap()).unwrap())
}
fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}
fn catalog_input_price(catalog: &ModelCatalog, key: &str) -> Decimal {
    catalog
        .entries
        .get(key)
        .and_then(|entry| entry.prices.values.input.input_token)
        .unwrap()
}
fn facts() -> CallFacts<'static> {
    CallFacts {
        call_id: "call-1",
        model: SMALL,
        provider: "openai",
    }
}
fn mock_call(entry: Entry) -> EmbeddingAnswer {
    let wire: EmbeddingCallWire = parse(
        r#"{"arguments": {"model": "text-embedding-3-small", "input": "hi", "mock_response": [0.5], "litellm_call_id": "call-1"}}"#,
    )
    .unwrap();
    get_runtime()
        .block_on(call(&session(), wire, entry))
        .unwrap()
}
// a request is complete once its headers ended and the content-length bytes after them arrived
fn complete(received: &[u8]) -> bool {
    let text = std::str::from_utf8(received).unwrap();
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0_usize);
    body.len() >= length
}
// one HTTP exchange on the local port: the request text it read, after writing the given answer
fn answer_once(listener: &TcpListener, status: &str, body: &str) -> String {
    let (mut stream, _peer) = listener.accept().unwrap();
    let mut received: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !complete(&received) {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0_usize, "the client closed before the request ended");
        received.extend_from_slice(&chunk[..read]);
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\nx-ratelimit-remaining-requests: 9\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).unwrap();
    String::from_utf8(received).unwrap()
}
fn served_call(
    status: &str,
    body: &str,
    api_key: &str,
) -> (String, Result<EmbeddingAnswer, CallFailure>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let wire: EmbeddingCallWire = parse(&format!(
        r#"{{"arguments": {{"model": "text-embedding-3-small", "input": "hi", "api_key": "{api_key}", "api_base": "http://{address}/v1", "timeout": 5, "headers": {{"x-custom": "one"}}}}}}"#
    ))
    .unwrap();
    let embedding_session = session();
    thread::scope(|scope| {
        let server = scope.spawn(|| answer_once(&listener, status, body));
        let outcome = get_runtime().block_on(call(&embedding_session, wire, Entry::Sync));
        // a call that failed before it connected would leave the server waiting in accept
        let _unblock = TcpStream::connect(address);
        (server.join().unwrap(), outcome)
    })
}
fn arguments(text: &str) -> EmbeddingArguments {
    parse(text).unwrap()
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
    assert_eq!(call.input_cost_per_token, Some(Decimal::new(1_i64, 1_u32)));
    assert_eq!(call.output_cost_per_token, Some(Decimal::new(2_i64, 1_u32)));
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
    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "type",
            "message",
            "status_code",
            "llm_provider",
            "model",
            "body",
            "headers",
            "num_retries",
            "max_retries",
        ]
    );
}
#[test]
fn request_build_moves_named_values_into_typed_fields() {
    let request = built(
        r#"{"input": "hi", "dimensions": 2, "user": "reader", "encoding_format": "float"}"#,
        SMALL,
        &[],
    )
    .unwrap();
    assert_eq!(request.model.as_deref(), Some(SMALL));
    assert_eq!(request.input, EmbeddingInput::Text(String::from("hi")));
    assert_eq!(request.dimensions, Some(2_u32));
    assert_eq!(request.user.as_deref(), Some("reader"));
    assert_eq!(request.encoding_format, Some(EncodingFormat::Float));
    assert!(request.passthrough.is_empty());
}
#[test]
fn request_build_defaults_a_missing_input_to_no_texts() {
    let request = built("{}", SMALL, &[]).unwrap();
    assert_eq!(request.input, EmbeddingInput::Texts(Vec::new()));
    assert_eq!(request.dimensions, None);
    assert_eq!(request.user, None);
    assert_eq!(request.encoding_format, None);
}
#[rstest]
#[case::explicit_float(r#"{"encoding_format": "float"}"#, &[], Some(EncodingFormat::Float))]
#[case::explicit_base64(r#"{"encoding_format": "base64"}"#, &[], Some(EncodingFormat::Base64))]
#[case::environment_default("{}", &[("LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT", "base64")], Some(EncodingFormat::Base64))]
#[case::explicit_wins(r#"{"encoding_format": "float"}"#, &[("LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT", "base64")], Some(EncodingFormat::Float))]
#[case::empty_falls_back(r#"{"encoding_format": ""}"#, &[("LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT", "base64")], Some(EncodingFormat::Base64))]
#[case::environment_none("{}", &[("LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT", " None ")], None)]
#[case::environment_empty("{}", &[("LITELLM_DEFAULT_EMBEDDING_ENCODING_FORMAT", "")], None)]
fn request_encoding_format_resolves_in_order(
    #[case] text: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Option<EncodingFormat>,
) {
    let request = built(text, SMALL, pairs).unwrap();
    assert_eq!(request.encoding_format, expected);
    assert!(request.passthrough.is_empty());
}
#[test]
fn request_an_unknown_encoding_stays_a_body_field() {
    let request = built(r#"{"encoding_format": "binary"}"#, SMALL, &[]).unwrap();
    assert_eq!(request.encoding_format, None);
    let body = serde_json::to_string(&request).unwrap();
    assert!(body.contains(r#""encoding_format":"binary""#), "{body}");
}
#[test]
fn request_extra_body_and_unknown_kwargs_join_the_body() {
    let request = built(
        r#"{"input": "hi", "extra_body": {"truncate": "END"}, "input_type": "query"}"#,
        SMALL,
        &[],
    )
    .unwrap();
    let body = serde_json::to_string(&request).unwrap();
    assert!(body.contains(r#""truncate":"END""#), "{body}");
    assert!(body.contains(r#""input_type":"query""#), "{body}");
    assert!(!body.contains("extra_body"), "{body}");
}
#[test]
fn error_dimensions_on_ada_raise_unsupported_params() {
    let failure = built(r#"{"input": "hi", "dimensions": 2}"#, ADA, &[]).unwrap_err();
    let payload = payload_of(failure);
    assert_eq!(payload.kind, "UnsupportedParamsError");
    assert_eq!(payload.status_code, Some(400_u16));
    assert!(
        payload
            .message
            .starts_with("Setting dimensions is not supported")
    );
}
#[test]
fn request_dimensions_on_ada_drop_with_drop_params() {
    let request = built(
        r#"{"input": "hi", "dimensions": 2, "drop_params": true}"#,
        ADA,
        &[],
    )
    .unwrap();
    assert_eq!(request.dimensions, None);
}
#[test]
fn request_dimensions_on_ada_drop_with_the_module_setting() {
    let request = build(
        &arguments(r#"{"input": "hi", "dimensions": 2}"#),
        ADA,
        "openai",
        true,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(request.dimensions, None);
}
#[test]
fn request_custom_prices_need_both_halves() {
    let prices = catalog();
    let both = arguments(r#"{"input_cost_per_token": 0.0001, "output_cost_per_token": 0.0002}"#);
    let input_only = arguments(r#"{"input_cost_per_token": 0.0001}"#);
    let custom = priced(Decimal::new(1_i64, 4_u32), 2_u32);
    let listed = priced(catalog_input_price(&prices, SMALL), 2_u32);
    assert_eq!(
        response_cost(&both, &prices, SMALL, "openai", 2_u32),
        custom
    );
    assert_eq!(
        response_cost(&input_only, &prices, SMALL, "openai", 2_u32),
        listed
    );
}
#[test]
fn request_catalog_price_falls_back_to_the_provider_prefix() {
    let prices = catalog();
    let listed = priced(catalog_input_price(&prices, "mistral/mistral-embed"), 3_u32);
    let cost = response_cost(&arguments("{}"), &prices, "mistral-embed", "mistral", 3_u32);
    assert_eq!(cost, listed);
}
#[test]
fn request_an_unpriced_model_costs_null() {
    let prices = catalog();
    let cost = response_cost(&arguments("{}"), &prices, "no-such-model", "openai", 3_u32);
    assert_eq!(cost, Json::Null);
}
#[test]
fn request_hidden_params_skip_provider_and_overhead_when_absent() {
    let facts = HiddenFacts {
        call_id: "call-1",
        api_base: "http://base",
        litellm_model: SMALL,
        provider: None,
    };
    let hidden = hidden_params(&facts, Json::Null, AdditionalHeaders::new(), 12.5_f64, None);
    assert!(!hidden.contains_key("custom_llm_provider"));
    assert!(!hidden.contains_key("litellm_overhead_time_ms"));
    assert_eq!(hidden.get("model_id"), Some(&Json::Null));
    assert_eq!(
        hidden.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-1")))
    );
    assert_eq!(
        hidden.get("api_base"),
        Some(&Json::Text(String::from("http://base")))
    );
    assert_eq!(
        hidden.get("litellm_model_name"),
        Some(&Json::Text(String::from(SMALL)))
    );
    assert_eq!(
        hidden.get("_response_ms").map(text_of).as_deref(),
        Some("12.5")
    );
    assert_eq!(
        hidden.get("callback_duration_ms").map(text_of).as_deref(),
        Some("0.0")
    );
}
#[test]
fn request_hidden_params_carry_provider_and_overhead_when_given() {
    let facts = HiddenFacts {
        call_id: "call-1",
        api_base: "http://base",
        litellm_model: SMALL,
        provider: Some("openai"),
    };
    let cost = Json::Number(Number::from(1_u32));
    let hidden = hidden_params(
        &facts,
        cost,
        AdditionalHeaders::new(),
        12.5_f64,
        Some(2.5_f64),
    );
    assert_eq!(
        hidden.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(
        hidden
            .get("litellm_overhead_time_ms")
            .map(text_of)
            .as_deref(),
        Some("2.5")
    );
    assert_eq!(
        hidden.get("response_cost").map(text_of).as_deref(),
        Some("1")
    );
}
#[test]
fn request_mock_usage_keeps_total_tokens_at_zero() {
    assert_eq!(
        text_of(&mock_usage()),
        r#"{"prompt_tokens":10,"completion_tokens":0,"total_tokens":0}"#
    );
}
#[rstest]
#[case::floats(EmbeddingVector::Floats(vec![Number::from(1_u32)]), r#"{"object":"embedding","index":3,"embedding":[1]}"#)]
#[case::base64(
    EmbeddingVector::Base64(String::from("AAAA")),
    r#"{"object":"embedding","index":3,"embedding":"AAAA"}"#
)]
fn request_records_keep_the_vector_shape(#[case] vector: EmbeddingVector, #[case] expected: &str) {
    let record = EmbeddingRecord {
        object: String::from("embedding"),
        embedding: vector,
        index: 3_u32,
    };
    assert_eq!(text_of(&record_json(&record)), expected);
}
#[rstest]
#[case::empty(None, None)]
#[case::blank(Some(""), None)]
#[case::given(Some("sk-key"), Some("sk-key"))]
fn request_secret_text_treats_empty_as_unset(
    #[case] given: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let secret = given.map(SecretString::from);
    let kept = secret_text(secret.as_ref());
    assert_eq!(kept.as_ref().map(ExposeSecret::expose_secret), expected);
}
#[rstest]
#[case::argument(r#"{"api_base": "http://argument"}"#, r#"{"api_base": "http://module"}"#, &[("OPENAI_BASE_URL", "http://url")], "http://argument")]
#[case::module(r#"{"api_base": ""}"#, r#"{"api_base": "http://module"}"#, &[("OPENAI_BASE_URL", "http://url")], "http://module")]
#[case::base_url("{}", r#"{"api_base": ""}"#, &[("OPENAI_BASE_URL", "http://url"), ("OPENAI_API_BASE", "http://api")], "http://url")]
#[case::api_base_env("{}", "{}", &[("OPENAI_BASE_URL", ""), ("OPENAI_API_BASE", "http://api")], "http://api")]
#[case::default("{}", "{}", &[], "https://api.openai.com")]
fn request_mock_api_base_follows_main_py(
    #[case] given: &str,
    #[case] module: &str,
    #[case] pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings: EmbeddingSettingsWire = parse(module).unwrap();
    let base = mock_api_base(&arguments(given), &settings, &environment(pairs));
    assert_eq!(base, expected);
}
#[test]
fn request_openai_defaults_skip_empty_environment_values() {
    let defaults = openai_defaults(&environment(&[
        ("OPENAI_BASE_URL", ""),
        ("OPENAI_API_BASE", "http://api"),
        ("OPENAI_API_KEY", "sk-env"),
        ("OPENAI_ORGANIZATION", "org-1"),
    ]));
    assert_eq!(defaults.api_bases, vec![String::from("http://api")]);
    let keys: Vec<&str> = defaults
        .api_keys
        .iter()
        .map(ExposeSecret::expose_secret)
        .collect();
    assert_eq!(keys, ["sk-env"]);
    assert_eq!(defaults.organization.as_deref(), Some("org-1"));
    let empty = openai_defaults(&environment(&[("OPENAI_API_KEY", "")]));
    assert!(empty.api_keys.is_empty());
    assert!(empty.api_bases.is_empty());
}
#[test]
fn request_response_headers_keep_text_values_and_project_rate_limits() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-ratelimit-remaining-requests",
        HeaderValue::from_static("9"),
    );
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    headers.insert("x-binary", HeaderValue::from_bytes(b"\xff").unwrap());
    let collected = header_map(&headers);
    assert_eq!(collected.len(), 2_usize);
    assert_eq!(
        collected.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
    let projected = additional_headers(&collected);
    assert!(projected.contains_key("x-ratelimit-remaining-requests"));
    assert!(projected.contains_key("llm_provider-x-ratelimit-remaining-requests"));
    assert!(projected.contains_key("llm_provider-x-request-id"));
}
#[test]
fn request_a_session_needs_its_settings_text() {
    EmbeddingSession::build("not json").err().unwrap();
    let built_session = session();
    assert!(built_session.environment.is_empty());
}
#[rstest]
#[case::bad_request(Raised::BadRequest, "BadRequestError", Some(400_u16))]
#[case::internal_server(Raised::InternalServer, "InternalServerError", Some(500_u16))]
#[case::timeout(Raised::Timeout, "Timeout", Some(408_u16))]
#[case::unsupported_params(Raised::UnsupportedParams, "UnsupportedParamsError", Some(400_u16))]
#[case::plain(Raised::Plain, "Exception", None)]
fn error_raised_classes_keep_their_status(
    #[case] kind: Raised,
    #[case] name: &str,
    #[case] status: Option<u16>,
) {
    let payload = payload_of(raised(
        kind,
        String::from("boom"),
        Some(String::from(SMALL)),
        None,
    ));
    assert_eq!(payload.kind, name);
    assert_eq!(payload.status_code, status);
    assert_eq!(payload.message, "boom");
    assert_eq!(payload.model.as_deref(), Some(SMALL));
}
#[rstest]
#[case::missing_key(OperationError::MissingApiKey, "InternalServerError", Some(500_u16))]
#[case::timeout(OperationError::Timeout, "Timeout", Some(408_u16))]
#[case::no_adapter(OperationError::UnavailableAdapter { provider: String::from("openai") }, "InternalServerError", Some(500_u16))]
fn error_operation_failures_map_to_litellm_classes(
    #[case] error: OperationError,
    #[case] name: &str,
    #[case] status: Option<u16>,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let payload = payload_of(operation_failure(error, &facts(), &redactor));
    assert_eq!(payload.kind, name);
    assert_eq!(payload.status_code, status);
    assert_eq!(payload.model.as_deref(), Some(SMALL));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
}
#[test]
fn error_a_provider_401_is_the_authentication_class() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let response = UpstreamResponse {
        status: 401_u16,
        headers: HeaderMap::new(),
        body: UNAUTHORIZED_BODY.as_bytes().to_vec(),
    };
    let error = OperationError::Provider {
        response: Box::new(response),
    };
    let payload = payload_of(operation_failure(error, &facts(), &redactor));
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401_u16));
    assert!(
        payload.message.contains("Invalid API key provided"),
        "{}",
        payload.message
    );
}
#[test]
fn error_an_unknown_model_is_a_bad_request() {
    let wire: EmbeddingCallWire =
        parse(r#"{"arguments": {"model": "no-such-model", "input": "hi"}}"#).unwrap();
    let failure = get_runtime()
        .block_on(call(&session(), wire, Entry::Sync))
        .unwrap_err();
    let payload = payload_of(failure);
    assert_eq!(payload.kind, "BadRequestError");
    assert_eq!(payload.model.as_deref(), Some("no-such-model"));
}
#[test]
fn error_a_mock_error_text_is_a_plain_exception() {
    let wire: EmbeddingCallWire = parse(
        r#"{"arguments": {"model": "text-embedding-3-small", "input": "hi", "mock_response": "error"}}"#,
    )
    .unwrap();
    let failure = get_runtime()
        .block_on(call(&session(), wire, Entry::Sync))
        .unwrap_err();
    let payload = payload_of(failure);
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.message, "Mock error");
}
#[test]
fn request_a_mock_answer_skips_the_provider() {
    let answer = mock_call(Entry::Sync);
    assert_eq!(answer.object, "list");
    assert_eq!(answer.model, SMALL);
    assert_eq!(
        answer.data.iter().map(text_of).collect::<Vec<String>>(),
        [r#"{"object":"embedding","index":0,"embedding":[0.5]}"#]
    );
    assert_eq!(
        text_of(&answer.usage),
        r#"{"prompt_tokens":10,"completion_tokens":0,"total_tokens":0}"#
    );
    assert!(answer.response_headers.is_empty());
    assert!(!answer.hidden_params.contains_key("custom_llm_provider"));
    assert!(
        !answer
            .hidden_params
            .contains_key("litellm_overhead_time_ms")
    );
    assert_eq!(
        answer.hidden_params.get("api_base"),
        Some(&Json::Text(String::from("https://api.openai.com")))
    );
    assert_ne!(answer.hidden_params.get("response_cost"), Some(&Json::Null));
}
#[test]
fn request_an_awaited_mock_answer_names_the_provider() {
    let answer = mock_call(Entry::Async);
    assert_eq!(
        answer.hidden_params.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
}
#[test]
fn request_a_provider_answer_carries_headers_and_hidden_params() {
    let (sent, outcome) = served_call("200 OK", EMBEDDING_BODY, "sk-test");
    let answer = outcome.unwrap();
    assert!(sent.starts_with("POST /v1/embeddings "), "{sent}");
    assert!(sent.contains("x-custom: one"), "{sent}");
    assert!(
        sent.to_lowercase()
            .contains("authorization: bearer sk-test"),
        "{sent}"
    );
    assert!(sent.contains(r#""input":"hi""#), "{sent}");
    assert_eq!(answer.model, SMALL);
    assert_eq!(
        answer.data.iter().map(text_of).collect::<Vec<String>>(),
        [r#"{"object":"embedding","index":0,"embedding":[0.1,0.2]}"#]
    );
    assert_eq!(
        answer
            .response_headers
            .get("x-ratelimit-remaining-requests")
            .map(String::as_str),
        Some("9")
    );
    let hidden = &answer.hidden_params;
    assert_eq!(
        hidden.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    let Some(Json::Object(projected)) = hidden.get("additional_headers") else {
        panic!("the hidden params carry no additional headers");
    };
    assert!(projected.contains_key("llm_provider-x-ratelimit-remaining-requests"));
    let Some(Json::Number(overhead)) = hidden.get("litellm_overhead_time_ms") else {
        panic!("a provider answer carries its overhead");
    };
    let Some(Json::Number(total)) = hidden.get("_response_ms") else {
        panic!("a provider answer carries its response time");
    };
    assert!(overhead.as_f64() <= total.as_f64());
    assert_ne!(hidden.get("response_cost"), Some(&Json::Null));
}
#[test]
fn error_a_served_401_raises_the_authentication_class() {
    let (_sent, outcome) = served_call("401 Unauthorized", UNAUTHORIZED_BODY, "bad-key");
    let payload = payload_of(outcome.unwrap_err());
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401_u16));
    assert_eq!(payload.model.as_deref(), Some(SMALL));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
}
#[rstest]
#[case::azure_flag(r#"{"model": "text-embedding-3-small", "azure": true}"#, "azure")]
#[case::azure_flag_beats_the_provider(
    r#"{"model": "text-embedding-3-small", "azure": true, "custom_llm_provider": "openai"}"#,
    "azure"
)]
#[case::azure_false_keeps_the_provider(
    r#"{"model": "text-embedding-3-small", "azure": false, "custom_llm_provider": "azure"}"#,
    "azure"
)]
#[case::no_flag_infers_openai(r#"{"model": "text-embedding-3-small"}"#, "openai")]
#[case::azure_false_infers_openai(
    r#"{"model": "text-embedding-3-small", "azure": false}"#,
    "openai"
)]
fn request_the_azure_flag_routes_resolution(#[case] text: &str, #[case] provider: &str) {
    let resolved = session()
        .resolve(&arguments(text), None, &GlobalCredentials::default())
        .unwrap();
    assert_eq!(resolved.provider.slug(), provider);
}
#[test]
fn request_an_empty_api_base_counts_as_unset() {
    let resolved = session()
        .resolve(
            &arguments(r#"{"model": "text-embedding-3-small", "api_base": ""}"#),
            None,
            &GlobalCredentials::default(),
        )
        .unwrap();
    assert_eq!(resolved.api_base, None);
}
