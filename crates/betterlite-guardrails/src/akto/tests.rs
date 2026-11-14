#![cfg(test)]

#[path = "tests/config.rs"]
mod config;
#[path = "tests/transport.rs"]
mod transport;

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use betterlite_config::sections::akto_mirroring::{Configured, UnreachableFallback};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use rstest::rstest;
use secrecy::SecretString;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt;

use super::AktoMirroring;
use super::error::AktoError;
use super::payload::{AktoInputs, AktoPayload};
use super::transport::{AktoAnswer, build_query_params, prepare_headers};

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object(entries: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn array(values: Vec<GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Array(values)
}

fn configured() -> Configured {
    Configured {
        guardrail_name: "akto".to_owned(),
        api_base: "http://akto.test".to_owned(),
        api_key: SecretString::from("secret"),
        account_id: "1000000".to_owned(),
        vxlan_id: "0".to_owned(),
        unreachable_fallback: UnreachableFallback::FailClosed,
        timeout: Duration::from_secs(5),
    }
}

fn guardrail() -> AktoMirroring {
    AktoMirroring::new(configured(), reqwest::Client::default()).expect("guardrail")
}

fn fixed_clock() -> i64 {
    1_700_000_000_000
}

fn guardrail_at(clock: fn() -> i64) -> AktoMirroring {
    let mut guardrail = guardrail();
    guardrail.now_millis = clock;
    guardrail
}

fn inputs() -> AktoInputs {
    AktoInputs {
        texts: vec!["hello".to_owned()],
        model: Some("gpt-4o".to_owned()),
        tools: None,
        tool_calls: None,
        structured_messages: None,
    }
}

fn payload_fields(payload: &AktoPayload) -> BTreeMap<String, GuardrailValue> {
    let encoded = serde_json::to_string(payload).expect("payload json");
    serde_json::from_str(&encoded).expect("payload value")
}

fn string_field(fields: &BTreeMap<String, GuardrailValue>, name: &str) -> String {
    match fields.get(name) {
        Some(GuardrailValue::Text(text)) => text.clone(),
        other => panic!("{name} is not text: {other:?}"),
    }
}

fn decode(encoded: &str) -> GuardrailValue {
    serde_json::from_str(encoded).expect("decoded json")
}

fn decode_double(encoded: &str) -> GuardrailValue {
    let GuardrailValue::Object(wrapper) = decode(encoded) else {
        panic!("the encoded body is not an object");
    };
    let Some(GuardrailValue::Text(inner)) = wrapper.get("body") else {
        panic!("the encoded body has no inner body text");
    };
    decode(inner)
}

#[test]
fn akto_guardrail_payload_double_encodes_request_and_response() {
    let payload = guardrail_at(fixed_clock)
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, true)
        .expect("payload");
    let fields = payload_fields(&payload);
    assert_eq!(
        decode_double(&string_field(&fields, "requestPayload")),
        object(vec![
            ("model", text("gpt-4o")),
            (
                "messages",
                array(vec![object(vec![
                    ("role", text("user")),
                    ("content", text("hello")),
                ])]),
            ),
        ])
    );
    assert_eq!(
        decode_double(&string_field(&fields, "responsePayload")),
        object(vec![(
            "choices",
            array(vec![object(vec![(
                "message",
                object(vec![
                    ("content", text("hello")),
                    ("role", text("assistant")),
                ]),
            )])]),
        )])
    );
    assert_eq!(string_field(&fields, "time"), "1700000000000".to_owned());
    assert_eq!(string_field(&fields, "method"), "POST");
    assert_eq!(
        string_field(&fields, "requestHeaders"),
        "{\"content-type\":\"application/json\"}"
    );
}

#[test]
fn akto_guardrail_payload_leaves_an_empty_response_without_include_response() {
    let payload = guardrail()
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let fields = payload_fields(&payload);
    assert_eq!(string_field(&fields, "responsePayload"), "{}");
    assert_eq!(string_field(&fields, "responseHeaders"), "{}");
    assert_eq!(
        decode_double(&string_field(&fields, "requestPayload")),
        object(vec![
            ("model", text("gpt-4o")),
            (
                "messages",
                array(vec![object(vec![
                    ("role", text("user")),
                    ("content", text("hello")),
                ])]),
            ),
        ])
    );
}

#[rstest]
#[case::route_metadata(Some("/v1/responses"), "/v1/responses")]
#[case::missing_route(None, "/v1/chat/completions")]
#[case::empty_route(Some(""), "/v1/chat/completions")]
#[case::non_text_route(None, "/v1/chat/completions")]
fn akto_guardrail_payload_uses_the_route_metadata(
    #[case] route: Option<&str>,
    #[case] expected: &str,
) {
    let mut request_data = BTreeMap::new();
    if let Some(route) = route {
        request_data.insert(
            "metadata".to_owned(),
            object(vec![("user_api_key_request_route", text(route))]),
        );
    } else {
        request_data.insert(
            "metadata".to_owned(),
            object(vec![(
                "user_api_key_request_route",
                GuardrailValue::SignedInteger(3),
            )]),
        );
    }
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        string_field(&payload_fields(&payload), "path"),
        expected.to_owned()
    );
}

#[rstest]
#[case::forwarded_for_list("203.0.113.7, 10.0.0.1", None, "203.0.113.7")]
#[case::forwarded_for_spaces("203.0.113.7 , 10.0.0.1", None, "203.0.113.7")]
#[case::real_ip_fallback("", Some("198.51.100.9"), "198.51.100.9")]
#[case::no_forwarded_for(None, Some("198.51.100.9"), "198.51.100.9")]
#[case::no_comma_is_not_trimmed(" 203.0.113.7 ", None, " 203.0.113.7 ")]
#[case::no_headers(None, None, "")]
fn akto_guardrail_payload_extracts_the_client_ip(
    #[case] forwarded_for: Option<&str>,
    #[case] real_ip: Option<&str>,
    #[case] expected: &str,
) {
    let mut headers = BTreeMap::new();
    if let Some(value) = forwarded_for {
        headers.insert("x-forwarded-for".to_owned(), text(value));
    }
    if let Some(value) = real_ip {
        headers.insert("x-real-ip".to_owned(), text(value));
    }
    let request_data = BTreeMap::from([(
        "proxy_server_request".to_owned(),
        object(vec![("headers", GuardrailValue::Object(headers))]),
    )]);
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        string_field(&payload_fields(&payload), "ip"),
        expected.to_owned()
    );
}

#[test]
fn akto_guardrail_payload_builds_tags_from_both_metadata_containers() {
    let request_data = BTreeMap::from([
        (
            "litellm_metadata".to_owned(),
            object(vec![("user_api_key_user_id", text("user-1"))]),
        ),
        (
            "metadata".to_owned(),
            object(vec![("user_api_key_team_id", text("team-1"))]),
        ),
    ]);
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, false)
        .expect("payload");
    let fields = payload_fields(&payload);
    let expected = object(vec![
        ("gen-ai", text("Gen AI")),
        ("user_id", text("user-1")),
        ("team_id", text("team-1")),
    ]);
    assert_eq!(decode(&string_field(&fields, "tag")), expected);
    assert_eq!(
        string_field(&fields, "tag"),
        string_field(&fields, "metadata")
    );
}

#[test]
fn akto_guardrail_payload_prefers_litellm_metadata_and_omits_blank_tags() {
    let request_data = BTreeMap::from([
        (
            "litellm_metadata".to_owned(),
            object(vec![("user_api_key_user_id", text(" first "))]),
        ),
        (
            "metadata".to_owned(),
            object(vec![
                ("user_api_key_user_id", text("second")),
                ("user_api_key_team_id", text("  ")),
            ]),
        ),
    ]);
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, false)
        .expect("payload");
    let tag = decode(&string_field(&payload_fields(&payload), "tag"));
    assert_eq!(
        tag,
        object(vec![("gen-ai", text("Gen AI")), ("user_id", text("first"))])
    );
}

#[test]
fn akto_guardrail_payload_prefers_structured_messages() {
    let inputs = AktoInputs {
        texts: vec!["hello".to_owned()],
        model: Some("gpt-4o".to_owned()),
        tools: None,
        tool_calls: None,
        structured_messages: Some(array(vec![object(vec![("role", text("system"))])])),
    };
    let request_data = BTreeMap::from([(
        "messages".to_owned(),
        array(vec![object(vec![("role", text("user"))])]),
    )]);
    let payload = guardrail()
        .build_akto_payload(&inputs, &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        decode_double(&string_field(&payload_fields(&payload), "requestPayload")),
        object(vec![
            ("model", text("gpt-4o")),
            (
                "messages",
                array(vec![object(vec![("role", text("system"))])])
            ),
        ])
    );
}

#[test]
fn akto_guardrail_payload_falls_back_to_request_messages_and_model() {
    let request_data = BTreeMap::from([
        (
            "messages".to_owned(),
            array(vec![object(vec![("role", text("user"))])]),
        ),
        ("model".to_owned(), text("request-model")),
    ]);
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        decode_double(&string_field(&payload_fields(&payload), "requestPayload")),
        object(vec![
            ("model", text("request-model")),
            (
                "messages",
                array(vec![object(vec![("role", text("user"))])])
            ),
        ])
    );
}

#[test]
fn akto_guardrail_payload_takes_tools_and_tool_calls_from_the_inputs() {
    let inputs = AktoInputs {
        texts: Vec::new(),
        model: None,
        tools: Some(array(vec![object(vec![("type", text("function"))])])),
        tool_calls: Some(array(vec![object(vec![("id", text("call-1"))])])),
        structured_messages: None,
    };
    let request_data = BTreeMap::from([(
        "tools".to_owned(),
        array(vec![object(vec![("type", text("request-tool"))])]),
    )]);
    let payload = guardrail()
        .build_akto_payload(&inputs, &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        decode_double(&string_field(&payload_fields(&payload), "requestPayload")),
        object(vec![
            ("model", text("")),
            ("messages", array(Vec::new())),
            (
                "tools",
                array(vec![object(vec![("type", text("function"))])])
            ),
            (
                "tool_calls",
                array(vec![object(vec![("id", text("call-1"))])]),
            ),
        ])
    );
}

#[test]
fn akto_guardrail_payload_uses_request_tools_when_the_inputs_have_none() {
    let inputs = AktoInputs {
        texts: Vec::new(),
        model: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
    };
    let request_data = BTreeMap::from([(
        "tools".to_owned(),
        array(vec![object(vec![("type", text("request-tool"))])]),
    )]);
    let payload = guardrail()
        .build_akto_payload(&inputs, &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        decode_double(&string_field(&payload_fields(&payload), "requestPayload")),
        object(vec![
            ("model", text("")),
            ("messages", array(Vec::new())),
            (
                "tools",
                array(vec![object(vec![("type", text("request-tool"))])])
            ),
        ])
    );
}

#[test]
fn akto_guardrail_payload_copies_request_headers_lowercased() {
    let request_data = BTreeMap::from([(
        "proxy_server_request".to_owned(),
        object(vec![(
            "headers",
            object(vec![
                ("Content-Type", text("text/plain")),
                ("X-Custom", text("abc")),
                ("X-Empty", text("")),
            ]),
        )]),
    )]);
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        decode(&string_field(&payload_fields(&payload), "requestHeaders")),
        object(vec![
            ("content-type", text("text/plain")),
            ("x-custom", text("abc")),
        ])
    );
}

#[test]
fn akto_guardrail_payload_carries_the_account_and_vxlan_ids() {
    let mut guardrail = guardrail();
    guardrail.account_id = "4242".to_owned();
    guardrail.vxlan_id = "7".to_owned();
    let payload = guardrail
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let fields = payload_fields(&payload);
    assert_eq!(string_field(&fields, "akto_account_id"), "4242");
    assert_eq!(string_field(&fields, "akto_vxlan_id"), "7");
    assert_eq!(string_field(&fields, "is_pending"), "false");
    assert_eq!(string_field(&fields, "source"), "MIRRORING");
    assert_eq!(string_field(&fields, "contextSource"), "AGENTIC");
    assert_eq!(string_field(&fields, "destIp"), "127.0.0.1");
    assert_eq!(fields.get("direction"), Some(&GuardrailValue::Null));
}

#[test]
fn akto_guardrail_payload_carries_the_status_code() {
    let payload = guardrail()
        .build_akto_payload(&inputs(), &BTreeMap::new(), 403, false)
        .expect("payload");
    let fields = payload_fields(&payload);
    assert_eq!(string_field(&fields, "statusCode"), "403");
    assert_eq!(string_field(&fields, "status"), "403");
}

#[test]
fn akto_guardrail_payload_stamps_the_current_time() {
    let payload = guardrail()
        .build_akto_payload(&inputs(), &BTreeMap::new(), 200, false)
        .expect("payload");
    let time = string_field(&payload_fields(&payload), "time");
    assert!(time.chars().all(|character| character.is_ascii_digit()));
    assert!(time.len() >= 12);
}

#[rstest]
#[case::zero_integer(GuardrailValue::SignedInteger(0), false)]
#[case::one_integer(GuardrailValue::SignedInteger(1), true)]
#[case::zero_unsigned(GuardrailValue::UnsignedInteger(0), false)]
#[case::one_unsigned(GuardrailValue::UnsignedInteger(1), true)]
#[case::zero_decimal(GuardrailValue::Decimal(0.0), false)]
#[case::one_decimal(GuardrailValue::Decimal(1.0), true)]
#[case::false_flag(GuardrailValue::Boolean(false), false)]
#[case::true_flag(GuardrailValue::Boolean(true), true)]
#[case::empty_text(GuardrailValue::Text(String::new()), false)]
#[case::filled_text(GuardrailValue::Text("x".to_owned()), true)]
#[case::empty_array(GuardrailValue::Array(Vec::new()), false)]
#[case::filled_array(
    GuardrailValue::Array(vec![GuardrailValue::Text("x".to_owned())]),
    true
)]
#[case::empty_object(GuardrailValue::Object(BTreeMap::new()), false)]
#[case::filled_object(
    GuardrailValue::Object(BTreeMap::from([("k".to_owned(), GuardrailValue::Null)])),
    true
)]
fn akto_guardrail_payload_skips_falsy_values(#[case] value: GuardrailValue, #[case] kept: bool) {
    let inputs = AktoInputs {
        texts: Vec::new(),
        model: None,
        tools: Some(value),
        tool_calls: None,
        structured_messages: None,
    };
    let payload = guardrail()
        .build_akto_payload(&inputs, &BTreeMap::new(), 200, false)
        .expect("payload");
    let decoded = decode_double(&string_field(&payload_fields(&payload), "requestPayload"));
    let GuardrailValue::Object(body) = decoded else {
        panic!("the request payload is not an object");
    };
    assert_eq!(body.contains_key("tools"), kept);
}

#[test]
fn akto_guardrail_payload_ignores_empty_request_values() {
    let request_data = BTreeMap::from([
        ("messages".to_owned(), array(Vec::new())),
        ("tools".to_owned(), array(Vec::new())),
    ]);
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, false)
        .expect("payload");
    assert_eq!(
        decode_double(&string_field(&payload_fields(&payload), "requestPayload")),
        object(vec![
            ("model", text("gpt-4o")),
            (
                "messages",
                array(vec![object(vec![
                    ("role", text("user")),
                    ("content", text("hello")),
                ])]),
            ),
        ])
    );
}

#[test]
fn akto_guardrail_payload_uses_the_answer_as_the_response_body() {
    let request_data = BTreeMap::from([(
        "response".to_owned(),
        object(vec![
            ("id", text("chatcmpl-1")),
            ("object", text("chat.completion")),
        ]),
    )]);
    let payload = guardrail()
        .build_akto_payload(&inputs(), &request_data, 200, true)
        .expect("payload");
    assert_eq!(
        decode_double(&string_field(&payload_fields(&payload), "responsePayload")),
        object(vec![
            ("id", text("chatcmpl-1")),
            ("object", text("chat.completion")),
        ])
    );
}

#[rstest]
#[case::allowed(
    r#"{"data":{"guardrailsResult":{"Allowed":true,"Reason":"fine"}}}"#,
    true,
    "fine"
)]
#[case::blocked(
    r#"{"data":{"guardrailsResult":{"Allowed":false,"Reason":"pii"}}}"#,
    false,
    "pii"
)]
#[case::missing_allowed(r#"{"data":{"guardrailsResult":{"Reason":"why"}}}"#, true, "why")]
#[case::null_allowed(
    r#"{"data":{"guardrailsResult":{"Allowed":null,"Reason":""}}}"#,
    false,
    ""
)]
#[case::missing_reason(r#"{"data":{"guardrailsResult":{"Allowed":false}}}"#, false, "")]
#[case::empty_object("{}", true, "")]
#[case::empty_data(r#"{"data":{}}"#, true, "")]
#[case::data_list(r#"{"data":[]}"#, true, "")]
#[case::guardrails_not_object(r#"{"data":{"guardrailsResult":"no"}}"#, true, "")]
fn akto_guardrail_response_parses_the_verdict(
    #[case] body: &str,
    #[case] allowed: bool,
    #[case] reason: &str,
) {
    let answer = AktoAnswer {
        status: StatusCode::OK,
        body: body.as_bytes().to_vec(),
    };
    let verdict = AktoMirroring::handle_guardrail_response(&answer).expect("verdict");
    assert_eq!(verdict.allowed, allowed);
    assert_eq!(verdict.reason, reason);
}

#[rstest]
#[case::array("[]")]
#[case::text(r#""ok""#)]
#[case::number("3")]
#[case::null("null")]
fn akto_guardrail_response_permits_non_object_json(#[case] body: &str) {
    let answer = AktoAnswer {
        status: StatusCode::OK,
        body: body.as_bytes().to_vec(),
    };
    let verdict = AktoMirroring::handle_guardrail_response(&answer).expect("verdict");
    assert!(verdict.allowed);
    assert!(verdict.reason.is_empty());
}

#[rstest]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR)]
#[case::created(StatusCode::CREATED)]
#[case::no_content(StatusCode::NO_CONTENT)]
fn akto_guardrail_response_rejects_a_non_200_status(#[case] status: StatusCode) {
    let answer = AktoAnswer {
        status,
        body: Vec::new(),
    };
    let error = AktoMirroring::handle_guardrail_response(&answer).expect_err("http status");
    assert!(matches!(error, AktoError::HttpStatus { status: reported } if reported == status));
}

#[test]
fn akto_guardrail_response_rejects_malformed_json() {
    let answer = AktoAnswer {
        status: StatusCode::OK,
        body: b"not json".to_vec(),
    };
    let error = AktoMirroring::handle_guardrail_response(&answer).expect_err("response json");
    assert!(matches!(&error, AktoError::ResponseJson { .. }));
}

#[test]
fn akto_guardrail_response_rejects_a_non_boolean_allowed() {
    let answer = AktoAnswer {
        status: StatusCode::OK,
        body: br#"{"data":{"guardrailsResult":{"Allowed":"false"}}}"#.to_vec(),
    };
    let error = AktoMirroring::handle_guardrail_response(&answer).expect_err("response field");
    assert!(
        matches!(error, AktoError::ResponseField { field } if field == "data.guardrailsResult.Allowed")
    );
}

#[test]
fn akto_guardrail_response_rejects_a_non_text_reason() {
    let answer = AktoAnswer {
        status: StatusCode::OK,
        body: br#"{"data":{"guardrailsResult":{"Allowed":false,"Reason":7}}}"#.to_vec(),
    };
    let error = AktoMirroring::handle_guardrail_response(&answer).expect_err("response field");
    assert!(
        matches!(error, AktoError::ResponseField { field } if field == "data.guardrailsResult.Reason")
    );
}

fn log_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("betterlite-akto-{}-{name}", std::process::id()))
}

fn log_to(path: &std::path::Path) -> tracing::subscriber::DefaultGuard {
    let writer = Arc::new(fs::File::create(path).expect("log file"));
    let subscriber = fmt::Subscriber::builder()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(writer)
        .finish();
    set_default(subscriber)
}

#[test]
fn akto_guardrail_unreachable_fails_open() {
    let mut guardrail = guardrail();
    guardrail.unreachable_fallback = UnreachableFallback::FailOpen;
    let path = log_path("fail-open");
    let returned = {
        let _guard = log_to(&path);
        guardrail
            .handle_unreachable(inputs(), &AktoError::Unreachable)
            .expect("passthrough")
    };
    assert_eq!(returned, inputs());
    let written = fs::read_to_string(&path).expect("log contents");
    fs::remove_file(&path).expect("remove log");
    assert!(written.contains("fail-open"));
}

#[test]
fn akto_guardrail_unreachable_fails_closed_with_503() {
    let mut guardrail = guardrail();
    guardrail.unreachable_fallback = UnreachableFallback::FailClosed;
    let path = log_path("fail-closed");
    let error = {
        let _guard = log_to(&path);
        guardrail
            .handle_unreachable(inputs(), &AktoError::Unreachable)
            .expect_err("rejected")
    };
    assert!(matches!(&error, AktoError::Unreachable));
    assert_eq!(error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
    let written = fs::read_to_string(&path).expect("log contents");
    fs::remove_file(&path).expect("log removal");
    assert!(written.contains("fail-closed"));
}

#[test]
fn akto_guardrail_requires_a_base_url() {
    let mut settings = configured();
    settings.api_base = String::new();
    let error = AktoMirroring::new(settings, reqwest::Client::default()).expect_err("base url");
    assert!(matches!(error, AktoError::MissingBaseUrl));
}

#[test]
fn akto_guardrail_requires_an_api_key() {
    let mut settings = configured();
    settings.api_key = SecretString::from("");
    let error = AktoMirroring::new(settings, reqwest::Client::default()).expect_err("api key");
    assert!(matches!(error, AktoError::MissingApiKey));
}

#[test]
fn akto_guardrail_headers_carry_the_content_type_and_api_key() {
    let headers = prepare_headers(&SecretString::from("top-secret")).expect("headers");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["authorization"], "top-secret");
    assert!(!format!("{headers:?}").contains("top-secret"));
}

#[test]
fn akto_guardrail_rejects_an_api_key_that_is_not_a_header_value() {
    let error = prepare_headers(&SecretString::from("bad\nkey")).expect_err("header");
    assert!(matches!(error, AktoError::InvalidHeader { .. }));
}

#[rstest]
#[case::bare(false, false, None, None)]
#[case::guardrails(true, false, Some("true"), None)]
#[case::ingest(false, true, None, Some("true"))]
#[case::both(true, true, Some("true"), Some("true"))]
fn akto_guardrail_query_params_follow_the_flags(
    #[case] guardrails: bool,
    #[case] ingest_data: bool,
    #[case] guardrails_param: Option<&str>,
    #[case] ingest_param: Option<&str>,
) {
    let params = build_query_params(guardrails, ingest_data);
    assert_eq!(
        params.get("akto_connector").map(String::as_str),
        Some("litellm")
    );
    assert_eq!(
        params.get("guardrails").map(String::as_str),
        guardrails_param
    );
    assert_eq!(params.get("ingest_data").map(String::as_str), ingest_param);
}
