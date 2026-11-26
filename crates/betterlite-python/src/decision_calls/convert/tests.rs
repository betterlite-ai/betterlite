#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::decisions_send::format::DecisionsOutput;
use betterlite_error::error::details::RateLimitDetails;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::openai::{
    DecisionAnswer, DecisionResponse, DecisionUsage, InputTokensDetails, OutputTokensDetails,
};
use betterlite_models::decisions::systemone::{
    DecisionAnswer as SystemOneAnswer, DecisionsResponse as SystemOneResponse,
};
use betterlite_models::decisions::{Extra, HiddenParams, Json, TokenCount};
use betterlite_models::sandbox::SandboxOutputError;
use betterlite_provider::e2b::E2bError;
use betterlite_provider::opensandbox::helpers::OpenSandboxError;
use betterlite_provider::opensandbox::lifecycle::LifecycleError;
use betterlite_provider::sandbox_dispatch::error::SandboxLifecycleError;
use betterlite_provider::sandbox_dispatch::request::SandboxContainer;
use betterlite_sdk::call_wrapper::error::plain_error;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret;
use serde_json::Number;

use super::super::error::SessionFailure;
use super::super::sandbox::container::container_of;
use super::{
    CallWire, ConvertError, NativeDecisionPayload, SessionWire, decisions_answer, encode, parse,
    requested_timeout,
};

const WIRE: &str = r#"{
    "operation": "decisions",
    "settings": {
        "api_base": "https://global.example",
        "api_key": "sk-global",
        "openai_key": "sk-openai",
        "organization": "org-1",
        "drop_params": true
    },
    "arguments": {
        "model": "gpt-4o",
        "input": "hello",
        "questions": [{"type": "predicate", "name": "spam", "instructions": "Is it spam?"}],
        "safety_identifier": "user-1",
        "api_key": "sk-call",
        "api_base": "https://call.example",
        "timeout": 1.5,
        "custom_llm_provider": "openai",
        "extra_headers": {"x-one": "1"},
        "params": {"state": {"a": 1}, "litellm_call_id": "call-1"}
    }
}"#;

const SYSTEMONE_WIRE: &str = r#"{
    "operation": "systemone",
    "arguments": {
        "model": "gpt-4o",
        "state": {"text": "hello"},
        "questions": {"spam": {"type": "noul", "instructions": "Is it spam?"}},
        "timeout": 2,
        "params": {"input": "wrong call", "litellm_call_id": "call-2"}
    }
}"#;

fn openai_response() -> DecisionResponse {
    DecisionResponse {
        model: String::from("gpt-4o"),
        answers: vec![DecisionAnswer::Predicate {
            name: Some(String::from("spam")),
            probability: Number::from_f64(0.9_f64).unwrap(),
        }],
        usage: DecisionUsage {
            input_tokens: TokenCount(Number::from(7_u32)),
            input_tokens_details: InputTokensDetails::default(),
            output_tokens: TokenCount(Number::from(3_u32)),
            output_tokens_details: OutputTokensDetails::default(),
            total_tokens: TokenCount(Number::from(10_u32)),
        },
        hidden_params: HiddenParams::default(),
        extra: Extra::new(),
    }
}

fn raised(failure: SessionFailure) -> NativeDecisionPayload {
    let SessionFailure::Raised(payload) = failure else {
        panic!("the failure is not a raised LiteLLM error");
    };
    *payload
}

#[test]
fn request_decisions_maps_global_settings() {
    let wire: CallWire = parse(WIRE).unwrap();
    let globals = wire.settings.llm_globals();
    assert_eq!(globals.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(
        globals.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-global")
    );
    assert_eq!(
        globals.openai_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-openai")
    );
    assert_eq!(globals.organization.as_deref(), Some("org-1"));
    let credentials = wire.settings.global_credentials();
    assert_eq!(
        credentials.api_base.as_deref(),
        Some("https://global.example")
    );
    assert!(credentials.xai_key.is_none());
    assert!(wire.settings.global_drop_params());
}

#[test]
fn request_decisions_maps_every_argument() {
    let wire: CallWire = parse(WIRE).unwrap();
    assert_eq!(wire.operation, "decisions");
    let args = wire.arguments.decisions().unwrap();
    assert_eq!(args.model, "gpt-4o");
    assert_eq!(args.input, Some(Json::Text(String::from("hello"))));
    let Some(Json::List(questions)) = &args.questions else {
        panic!("the questions stay the list the caller sent");
    };
    assert_eq!(questions.len(), 1);
    assert_eq!(args.safety_identifier.as_deref(), Some("user-1"));
    assert_eq!(
        args.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-call")
    );
    assert_eq!(args.api_base.as_deref(), Some("https://call.example"));
    assert_eq!(args.timeout, Some(Duration::from_millis(1500)));
    assert_eq!(args.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        args.extra_headers,
        Some(BTreeMap::from([(String::from("x-one"), String::from("1"))]))
    );
    assert_eq!(
        args.kwargs.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-1")))
    );
    assert_eq!(
        args.kwargs.get("state"),
        Some(&Json::Object(Extra::from([(
            String::from("a"),
            Json::Number(Number::from(1_u32)),
        )])))
    );
}

#[test]
fn request_systemone_maps_state_and_keeps_input_as_a_param() {
    let wire: CallWire = parse(SYSTEMONE_WIRE).unwrap();
    assert_eq!(wire.operation, "systemone");
    assert_eq!(
        wire.settings.llm_globals().api_base,
        None,
        "an absent settings block reads as no globals"
    );
    let args = wire.arguments.systemone().unwrap();
    assert_eq!(
        args.state,
        Some(Json::Object(Extra::from([(
            String::from("text"),
            Json::Text(String::from("hello")),
        )])))
    );
    assert_eq!(args.timeout, Some(Duration::from_secs(2)));
    assert_eq!(
        args.kwargs.get("input"),
        Some(&Json::Text(String::from("wrong call")))
    );
}

#[test]
fn request_settings_drop_params_only_counts_an_explicit_true() {
    let absent: CallWire = parse(r#"{"settings": {}}"#).unwrap();
    assert!(!absent.settings.global_drop_params());
    let disabled: CallWire = parse(r#"{"settings": {"drop_params": false}}"#).unwrap();
    assert!(!disabled.settings.global_drop_params());
    let enabled: CallWire = parse(r#"{"settings": {"drop_params": true}}"#).unwrap();
    assert!(enabled.settings.global_drop_params());
}

#[test]
fn request_timeout_rejects_unusable_seconds() {
    assert_eq!(requested_timeout(None).unwrap(), None);
    assert_eq!(
        requested_timeout(Some(0.0_f64)).unwrap(),
        Some(Duration::ZERO)
    );
    assert_eq!(
        requested_timeout(Some(2.5_f64)).unwrap(),
        Some(Duration::from_millis(2500))
    );
    assert!(matches!(
        requested_timeout(Some(-1.0_f64)),
        Err(ConvertError::Timeout { .. })
    ));
    assert!(matches!(
        requested_timeout(Some(f64::NAN)),
        Err(ConvertError::Timeout { .. })
    ));
}

#[test]
fn request_session_wire_reads_the_environment() {
    let empty: SessionWire = parse("{}").unwrap();
    assert_eq!(empty.environment, None);
    let given: SessionWire = parse(r#"{"environment": {"E2B_API_KEY": "sk-e2b"}}"#).unwrap();
    assert_eq!(
        given.environment,
        Some(BTreeMap::from([(
            String::from("E2B_API_KEY"),
            String::from("sk-e2b")
        )]))
    );
}

#[test]
fn request_parse_names_text_that_is_not_the_wire() {
    let error = parse::<CallWire>("not json").unwrap_err();
    assert!(matches!(error, ConvertError::Json { .. }));
}

#[test]
fn request_answer_flattens_the_response_and_the_hidden_params() {
    let mut response = openai_response();
    response.hidden_params.insert(
        String::from("model_id"),
        Json::Text(String::from("openai/gpt-4o")),
    );
    let text = decisions_answer(&DecisionsOutput::OpenAi(response)).unwrap();
    let answer: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = answer else {
        panic!("the answer is a JSON object");
    };
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text(String::from("gpt-4o")))
    );
    let usage: Json = serde_json::from_str(
        r#"{
            "input_tokens": 7,
            "input_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 0},
            "output_tokens": 3,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": 10
        }"#,
    )
    .unwrap();
    assert_eq!(fields.get("usage"), Some(&usage));
    assert_eq!(
        fields.get("_hidden_params"),
        Some(&Json::Object(Extra::from([(
            String::from("model_id"),
            Json::Text(String::from("openai/gpt-4o")),
        )])))
    );
    assert!(!fields.contains_key("hidden_params"));
}

#[test]
fn request_systemone_answer_lists_answers_as_a_map() {
    let mut response = SystemOneResponse {
        model: Some(String::from("gpt-4o")),
        answers: IndexMap::from([(
            String::from("spam"),
            SystemOneAnswer::Noul {
                noul: Number::from_f64(0.9_f64).unwrap(),
                extra: Extra::new(),
            },
        )]),
        usage: None,
        hidden_params: HiddenParams::default(),
        extra: Extra::new(),
    };
    response.hidden_params.insert(
        String::from("model_id"),
        Json::Text(String::from("openai/gpt-4o")),
    );
    let text = decisions_answer(&DecisionsOutput::SystemOne(response)).unwrap();
    let answer: Json = serde_json::from_str(&text).unwrap();
    let Json::Object(fields) = answer else {
        panic!("the answer is a JSON object");
    };
    let Some(Json::Object(answers)) = fields.get("answers") else {
        panic!("the System One answers are a map");
    };
    assert_eq!(answers.len(), 1);
    assert!(fields.contains_key("_hidden_params"));
}

#[test]
fn request_encode_reports_the_value_it_cannot_write() {
    let mut response = openai_response();
    response
        .extra
        .insert(String::from("blob"), Json::Object(Extra::new()));
    encode(&response).unwrap();
    let invalid = BTreeMap::from([(vec![1_u8], "value")]);
    assert_eq!(
        encode(&invalid).unwrap_err().to_string(),
        "the answer could not be written as JSON: key must be a string"
    );
}

#[test]
fn request_container_of_reads_a_bare_id() {
    let wire: CallWire =
        parse(r#"{"arguments": {"provider": "e2b", "container": "sbx-1"}}"#).unwrap();
    let container = container_of(&wire.arguments).unwrap();
    let SandboxContainer::E2b(handle) = container else {
        panic!("a bare id reads as an e2b handle");
    };
    assert_eq!(handle.id, "sbx-1");
    assert_eq!(handle.domain, None);
}

#[test]
fn request_container_of_reads_an_opensandbox_handle() {
    let wire: CallWire = parse(
        r#"{"arguments": {"provider": "opensandbox", "container": {
            "id": "sbx-2",
            "provider": "opensandbox",
            "hidden_params": {
                "api_base": "https://os.example",
                "api_key": "sk-os",
                "execd_endpoint": "https://execd.example",
                "execd_headers": {"x-token": "abc"},
                "use_server_proxy": true
            }
        }}}"#,
    )
    .unwrap();
    let container = container_of(&wire.arguments).unwrap();
    let SandboxContainer::OpenSandbox(handle) = container else {
        panic!("a handle with an execd endpoint reads as an opensandbox handle");
    };
    assert_eq!(handle.container.id, "sbx-2");
    assert_eq!(handle.execd_endpoint, "https://execd.example");
    assert!(handle.use_server_proxy);
    assert_eq!(
        handle.container.hidden_params.api_base.as_deref(),
        Some("https://os.example")
    );
    assert_eq!(
        handle
            .execd_headers
            .get("x-token")
            .map(ExposeSecret::expose_secret),
        Some("abc")
    );
}

#[test]
fn request_container_of_rejects_invalid_handles() {
    let wrong: CallWire = parse(r#"{"arguments": {"container": 7}}"#).unwrap();
    assert!(matches!(
        container_of(&wrong.arguments),
        Err(ConvertError::Container)
    ));
    let missing_id: CallWire =
        parse(r#"{"arguments": {"container": {"provider": "e2b"}}}"#).unwrap();
    assert!(matches!(
        container_of(&missing_id.arguments),
        Err(ConvertError::Container)
    ));
}

#[test]
fn error_payload_from_a_plain_exception_carries_no_status() {
    let payload = NativeDecisionPayload::from_class(&plain_error("boom"));
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.message, "boom");
    assert_eq!(payload.status_code, None);
    assert_eq!(payload.llm_provider, None);
    assert_eq!(payload.model, None);
    assert_eq!(payload.body, None);
    assert!(payload.headers.is_empty());
    let text = encode(&payload).unwrap();
    assert!(text.contains(r#""type":"Exception""#));
}

#[test]
fn error_rate_limit_payload_keeps_the_explicit_headers() {
    let class = ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: ErrorContext {
            message: String::from("slow down"),
            model: Some(String::from("gpt-4o")),
            provider: Some(String::from("openai")),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        },
        category: RateLimitCategory::VendorRateLimit,
        dimension: None,
        headers: BTreeMap::from([(String::from("retry-after"), String::from("1"))]),
    }));
    let payload = NativeDecisionPayload::from_class(&class);
    assert_eq!(payload.kind, "RateLimitError");
    assert_eq!(payload.message, "slow down");
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
    assert_eq!(payload.model.as_deref(), Some("gpt-4o"));
    assert_eq!(payload.status_code, Some(429));
    assert_eq!(
        payload.headers.get("retry-after").map(String::as_str),
        Some("1")
    );
}

#[test]
fn error_sandbox_misuse_raises_litellm_value_errors() {
    let missing_key = raised(SessionFailure::from(SandboxLifecycleError::E2b(
        E2bError::MissingApiKey,
    )));
    assert_eq!(missing_key.kind, "ValueError");
    assert_eq!(
        missing_key.message,
        "E2B API key not set. Set E2B_API_KEY or pass api_key=..."
    );

    let bare_id = raised(SessionFailure::from(
        SandboxLifecycleError::ContainerHandleRequired { provider: "e2b" },
    ));
    assert_eq!(bare_id.kind, "ValueError");
    assert_eq!(
        bare_id.message,
        "Cannot run code from a sandbox id alone. e2b secure sandboxes require the access token returned by acreate_sandbox; pass the ContainerHandle it returned instead of a bare sandbox id."
    );

    let token = raised(SessionFailure::from(SandboxLifecycleError::E2b(
        E2bError::MissingAccessToken,
    )));
    assert_eq!(token.message, bare_id.message);

    let unsupported = raised(SessionFailure::from(
        SandboxLifecycleError::UnsupportedProvider {
            provider: String::from("daytona"),
        },
    ));
    assert_eq!(unsupported.kind, "ValueError");
    assert_eq!(
        unsupported.message,
        String::from("Code execution is not supported for provider: daytona")
    );

    let missing_base = raised(SessionFailure::from(SandboxLifecycleError::OpenSandbox(
        LifecycleError::Sandbox(OpenSandboxError::MissingApiBase {
            variable: "OPEN_SANDBOX_API_BASE",
        }),
    )));
    assert_eq!(missing_base.kind, "ValueError");
    assert_eq!(
        missing_base.message,
        "OpenSandbox api_base is required. Pass api_base or set OPEN_SANDBOX_API_BASE."
    );
}

#[test]
fn error_sandbox_waits_time_out_as_litellm_timeout_errors() {
    let not_running = raised(SessionFailure::from(SandboxLifecycleError::OpenSandbox(
        LifecycleError::Sandbox(OpenSandboxError::NotRunning {
            sandbox_id: String::from("sbx-1"),
            timeout: Duration::from_secs(30),
        }),
    )));
    assert_eq!(not_running.kind, "TimeoutError");
    assert_eq!(
        not_running.message,
        "OpenSandbox sandbox sbx-1 was not Running within 30.0 seconds"
    );

    let not_ready = raised(SessionFailure::from(SandboxLifecycleError::OpenSandbox(
        LifecycleError::Sandbox(OpenSandboxError::EndpointNotReady {
            sandbox_id: String::from("sbx-1"),
            timeout: Duration::from_millis(1500),
            last: Box::new(OpenSandboxError::HttpStatus { status: 500 }),
        }),
    )));
    assert_eq!(not_ready.kind, "TimeoutError");
    assert_eq!(
        not_ready.message,
        "OpenSandbox execd endpoint for sbx-1 was not ready within 1.5 seconds"
    );
}

#[test]
fn error_other_sandbox_failures_stay_plain_exceptions() {
    let failure = raised(SessionFailure::from(SandboxLifecycleError::OpenSandbox(
        LifecycleError::Sandbox(OpenSandboxError::HttpStatus { status: 500 }),
    )));
    assert_eq!(failure.kind, "Exception");
    assert_eq!(
        failure.message,
        "the OpenSandbox API answered with status 500"
    );
    assert_eq!(failure.status_code, None);
}

#[test]
fn error_operation_names_the_operation_the_wire_carried() {
    let error = ConvertError::Operation {
        operation: String::from("chat"),
    };
    assert_eq!(
        error.to_string(),
        "the chat operation is not a decision call"
    );
}

#[rstest]
#[case::e2b(SandboxLifecycleError::E2b(E2bError::OutputTooLarge { limit: 10_485_760_usize }))]
#[case::opensandbox(SandboxLifecycleError::OpenSandbox(LifecycleError::Sandbox(
    OpenSandboxError::OutputTooLarge { limit: 10_485_760_usize },
)))]
fn error_output_limits_are_value_errors(#[case] error: SandboxLifecycleError) {
    let payload = raised(SessionFailure::from(error));
    assert_eq!(payload.kind, "ValueError");
    assert_eq!(
        payload.message,
        "the sandbox output exceeded 10485760 bytes; aborting to avoid unbounded memory use"
    );
}

#[test]
fn error_create_body_limit_is_a_value_error() {
    let payload = raised(SessionFailure::from(SandboxLifecycleError::OpenSandbox(
        LifecycleError::Body {
            source: SandboxOutputError::Exceeded {
                limit: 10_485_760_usize,
            },
        },
    )));
    assert_eq!(payload.kind, "ValueError");
    assert_eq!(
        payload.message,
        "sandbox output exceeded 10485760 bytes; aborting to avoid unbounded memory use"
    );
}

#[rstest]
#[case::failed("Failed")]
#[case::stopping("Stopping")]
#[case::terminated("Terminated")]
fn error_terminal_states_are_value_errors(#[case] state: &str) {
    let payload = raised(SessionFailure::from(SandboxLifecycleError::OpenSandbox(
        LifecycleError::Sandbox(OpenSandboxError::UnexpectedState {
            sandbox_id: String::from("sbx-1"),
            state: state.to_owned(),
        }),
    )));
    assert_eq!(payload.kind, "ValueError");
    assert_eq!(
        payload.message,
        format!("OpenSandbox sandbox sbx-1 entered {state}")
    );
}
