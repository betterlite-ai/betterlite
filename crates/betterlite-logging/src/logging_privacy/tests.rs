#![cfg(test)]

use betterlite_config::sections::logging_privacy::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use std::{fs, path::PathBuf};

use super::callbacks::{SpanScope, TrustedCallbacks, callback_metadata, validate_environment};
use super::internal::{Forwarding, Origin, Session, call_category, forward, is_unbilled};
use super::request::{Decision, PrivacyRequest, decide_request};
use super::snapshot::redact_user_api_key_info;
use super::{CallbackMetadataError, PrivacyContext};
use betterlite_privacy_openai::{Payload, SnapshotError};

#[rstest]
#[case::typed_metadata(test_typed_metadata, "", "")]
#[case::private_passthrough(
    test_snapshot_shape,
    r#"{"litellm_params":{"metadata":{"input":"campaign-7","text":"tag"}}}"#,
    r#"{"litellm_params":{"metadata":{"input":"","text":"redacted-by-litellm"}}}"#
)]
#[case::global(test_policy, r#"{"global":true}"#, "true")]
#[case::default_public(test_policy, "{}", "false")]
#[case::dynamic_over_global(
    test_policy,
    r#"{"global":true,"request":{"turn_off_message_logging":false}}"#,
    "false"
)]
#[case::unrecognized_string(
    test_policy,
    r#"{"request":{"turn_off_message_logging":"yes"}}"#,
    "false"
)]
#[case::unrecognized_global(
    test_policy,
    r#"{"global":true,"request":{"turn_off_message_logging":"invalid"}}"#,
    "true"
)]
#[case::dynamic_string(
    test_policy,
    r#"{"request":{"turn_off_message_logging":"true"}}"#,
    "true"
)]
#[case::dynamic_false_string(
    test_policy,
    r#"{"global":true,"request":{"turn_off_message_logging":" FaLsE "}}"#,
    "false"
)]
#[case::disable_header(test_policy, r#"{"global":true,"request":{"turn_off_message_logging":true,"metadata":{"headers":{"litellm-disable-message-redaction":true}}}}"#, "false")]
#[case::enable_header(
    test_policy,
    r#"{"request":{"metadata":{"headers":{"x-litellm-enable-message-redaction":true}}}}"#,
    "true"
)]
#[case::legacy_enable(
    test_policy,
    r#"{"request":{"metadata":{"headers":{"litellm-enable-message-redaction":true}}}}"#,
    "true"
)]
#[case::dynamic_over_enable(test_policy, r#"{"request":{"turn_off_message_logging":false,"metadata":{"headers":{"litellm-enable-message-redaction":true}}}}"#, "false")]
#[case::direct_precedence(test_policy, r#"{"request":{"turn_off_message_logging":false,"litellm_params":{"metadata":{"turn_off_message_logging":true}}}}"#, "false")]
#[case::preferred_metadata(test_policy, r#"{"request":{"litellm_metadata":{"turn_off_message_logging":false},"metadata":{"turn_off_message_logging":true}}}"#, "false")]
#[case::preferred_headers(test_policy, r#"{"global":true,"request":{"litellm_metadata":{},"metadata":{"headers":{"litellm-disable-message-redaction":true}}}}"#, "true")]
#[case::null_metadata_fallback(test_policy, r#"{"global":true,"request":{"litellm_metadata":null,"metadata":{"headers":{"litellm-disable-message-redaction":true}}}}"#, "false")]
#[case::nested_over_metadata(test_policy, r#"{"request":{"litellm_params":{"metadata":{"turn_off_message_logging":true}},"litellm_metadata":{"turn_off_message_logging":false}}}"#, "true")]
#[case::inherited_private(test_policy, r#"{"inherited":true,"request":{"turn_off_message_logging":false,"headers":{"litellm-disable-message-redaction":true}}}"#, "true")]
#[case::callback_private(test_policy, r#"{"callback":false}"#, "true")]
#[case::key_info_global(test_key_policy, r#"{"redact_user_api_key_info":false}"#, "global")]
#[case::key_info_request(
    test_key_policy,
    r#"{"metadata":{"redact_user_api_key_info":true}}"#,
    "request"
)]
#[case::metadata_debug(test_metadata_debug, "", "")]
#[case::unknown_result(test_unknown_result, "", "")]
#[case::deep_credentials(test_deep_credentials, "", "")]
#[case::empty_forward(test_empty_forward, "", "")]
#[case::child_stays_private(test_child, "", "")]
#[case::credential_shapes(test_credentials, "", "")]
#[case::per_callback_copies(test_copies, "", "")]
#[case::embedding_output(test_embedding, "", "")]
#[case::stream_independence(test_stream, "", "")]
#[case::response_independence(test_response, "", "")]
#[case::provider_metadata(test_provider_metadata, "", "")]
#[case::trusted_config_overlay(test_trusted_overlay, "", "")]
#[case::valid_langfuse(test_environment, "production-1_a", "true")]
#[case::empty_langfuse(test_environment, "", "false")]
#[case::reserved_langfuse(test_environment, "langfuse-prod", "false")]
#[case::uppercase_langfuse(test_environment, "Production", "false")]
#[case::valid_span_scope(test_span_scope, "llm_only", "true")]
#[case::invalid_span_scope(test_span_scope, "any", "false")]
#[case::management_free(test_billing, "get_responses", "true")]
#[case::inference_billed(test_billing, "completion", "false")]
#[case::background_billed(test_billing, "background", "false")]
#[case::poll_billed(test_billing, "poll", "false")]
#[case::session_ids(test_session, "", "")]
#[case::destination_preserved(test_blocked, "langsmith_base_url", "")]
#[case::attached_forward(test_forwarding, "attached", r#"{"user_api_key_auth":{"team":"a"},"route":"r","internal_call_origin":"autorouter_classifier"}"#)]
#[case::detached_forward(
    test_forwarding,
    "detached",
    r#"{"user_api_key_auth":{"team":"a"},"internal_call_origin":"autorouter_classifier"}"#
)]
#[case::dd_site(test_blocked, "dd_site", "")]
#[case::dd_api_key(test_blocked, "dd_api_key", "")]
#[case::dd_agent_host(test_blocked, "dd_agent_host", "")]
#[case::dd_agent_port(test_blocked, "dd_agent_port", "")]
#[case::gcs_bucket(test_blocked, "gcs_bucket_name", "")]
#[case::gcs_account(test_blocked, "gcs_path_service_account", "")]
#[case::newrelic_key(test_blocked, "newrelic_api_key", "")]
#[case::newrelic_region(test_blocked, "newrelic_region", "")]
#[case::signoz_endpoint(test_blocked, "signoz_ingestion_endpoint", "")]
#[case::signoz_key(test_blocked, "signoz_ingestion_key", "")]
#[case::environment_reference(
    test_rejected,
    r#"{"langsmith_project":"os.environ/PRIVATE"}"#,
    "environment"
)]
#[case::nested_environment(
    test_rejected,
    r#"{"langsmith_project":[{"value":"os.environ/PRIVATE"}]}"#,
    "environment"
)]
#[case::trusted_vars(test_rejected, r#"{"litellm_trusted_callback_vars":{}}"#, "trusted")]
#[case::internal_origin(
    test_rejected,
    r#"{"internal_call_origin":"shadow_eval_judge"}"#,
    "trusted"
)]
#[case::internal_flag(test_rejected, r#"{"is_internal_call":true}"#, "trusted")]
#[case::invalid_request_flag(test_invalid_request, r#"{"turn_off_message_logging":12}"#, "")]
#[case::invalid_key_info_flag(
    test_invalid_request,
    r#"{"metadata":{"redact_user_api_key_info":"yes"}}"#,
    ""
)]
#[case::invalid_dynamic_text(
    test_invalid_request,
    r#"{"turn_off_message_logging":"os.environ/PRIVATE"}"#,
    ""
)]
#[case::dynamic_callback_values(test_dynamic_params, r#"{"langsmith_project":"direct","turn_off_message_logging":false,"metadata":{"langsmith_project":"nested","turn_off_message_logging":true,"langfuse_prompt_version":2,"dd_site":"ignored"}}"#, r#"{"langsmith_project":"direct","langfuse_prompt_version":2,"turn_off_message_logging":false}"#)]
#[case::dynamic_nested_values(test_dynamic_params, r#"{"litellm_params":{"metadata":{"langsmith_project":"params"}},"litellm_metadata":{"langsmith_project":"preferred","langfuse_environment":"production"},"metadata":{"langsmith_project":"fallback","turn_off_message_logging":"true"}}"#, r#"{"langsmith_project":"params","langfuse_environment":"production","turn_off_message_logging":true}"#)]
#[case::dynamic_environment_error(
    test_dynamic_error,
    r#"{"metadata":{"langsmith_project":"os.environ/PRIVATE"}}"#,
    "langsmith_project"
)]
#[case::dynamic_flag_environment(
    test_dynamic_error,
    r#"{"turn_off_message_logging":"os.environ/PRIVATE"}"#,
    "turn_off_message_logging"
)]
#[case::callback_environment_valid(
    test_callback_environment,
    r#"{"langfuse_environment":"production"}"#,
    "valid"
)]
#[case::callback_environment_invalid(
    test_callback_environment,
    r#"{"langfuse_environment":"Production"}"#,
    "invalid"
)]
#[case::wrapped_chat_result(
    test_wrapped_result,
    r#"{"choices":[{"message":{"content":"private"}}],"id":"chat"}"#,
    r#"{"choices":[{"message":{"content":"redacted-by-litellm"}}],"id":"chat"}"#
)]
#[case::wrapped_output_result(
    test_wrapped_result,
    r#"{"output":[],"id":"output"}"#,
    r#"{"output":[],"id":"output"}"#
)]
#[case::wrapped_embedding_result(
    test_wrapped_result,
    r#"{"object":"list","data":[{"embedding":[1]}],"model":"m"}"#,
    r#"{"object":"list","data":[],"model":"m"}"#
)]
#[case::wrapped_unknown_object(
    test_wrapped_result,
    r#"{"object":"unknown","private":"value"}"#,
    r#"{"text":"redacted-by-litellm"}"#
)]
#[case::non_embedding_data(
    test_snapshot_shape,
    r#"{"object":"other","data":[1]}"#,
    r#"{"object":"other","data":[1]}"#
)]
#[case::embedding_without_data(test_snapshot_shape, r#"{"object":"list"}"#, r#"{"object":"list"}"#)]
#[case::credential_depth_boundary(test_depth_boundary, "100", "public")]
#[case::credential_depth_exceeded(test_depth_boundary, "101", "REDACTED")]
fn logging_privacy_tests_behavior(
    #[case] check: fn(&str, &str),
    #[case] input: &str,
    #[case] expected: &str,
) {
    check(input, expected);
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PolicyCase {
    request: PrivacyRequest,
    global: bool,
    inherited: bool,
    callback: Option<bool>,
}

fn test_policy(input: &str, expected: &str) {
    let case: PolicyCase = serde_json::from_str(input).unwrap();
    let decision = case
        .request
        .decide(
            Settings {
                turn_off_message_logging: case.global,
                redact_user_api_key_info: false,
            },
            PrivacyContext {
                message_logging_disabled: case.inherited,
            },
            case.callback.unwrap_or(true),
        )
        .unwrap();
    assert_eq!(decision.content.to_string(), expected);
}

fn test_copies(_input: &str, _expected: &str) {
    let request = ChatRequest::parse(
        br#"{"messages":[{"role":"user","content":"hello sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#,
    )
    .unwrap();
    let metadata: Json =
        serde_json::from_str(r#"{"user_api_key_hash":"hash","api_key":"secret","team":"a"}"#)
            .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (public, public_metadata) = test_snapshot(
        &request,
        &metadata,
        Decision {
            content: false,
            key_info: false,
        },
        &redactor,
    )
    .unwrap();
    let (private, private_metadata) = test_snapshot(
        &request,
        &metadata,
        Decision {
            content: true,
            key_info: true,
        },
        &redactor,
    )
    .unwrap();
    assert_ne!(public, private);
    let public_text = serde_json::to_string(&public).unwrap();
    assert!(public_text.contains("hello"));
    assert!(!public_text.contains("sk-aaaa"));
    assert!(!serde_json::to_string(&private).unwrap().contains("hello"));
    assert!(serde_json::to_string(&request).unwrap().contains("sk-aaaa"));
    let public_meta = serde_json::to_string(&public_metadata).unwrap();
    let private_meta = serde_json::to_string(&private_metadata).unwrap();
    assert!(!public_meta.contains("secret"));
    assert!(!private_meta.contains("user_api_key"));
}

fn test_embedding(_input: &str, _expected: &str) {
    let response: EmbeddingResponse = serde_json::from_str(r#"{"object":"list","model":"m","data":[{"object":"embedding","index":0,"embedding":[1]}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (private, _) = test_snapshot(
        &response,
        &Json::Null,
        Decision {
            content: true,
            key_info: false,
        },
        &redactor,
    )
    .unwrap();
    let Json::Object(fields) = private else {
        panic!("object snapshot")
    };
    assert_eq!(fields.get("data"), Some(&Json::List(Vec::new())));
    assert_eq!(response.data.len(), 1);
    assert!(fields.contains_key("usage"));
}

fn test_blocked(input: &str, _expected: &str) {
    let payload: Json = serde_json::from_str(&format!(
        r#"{{"{input}":"os.environ/PRIVATE","langsmith_project":"project"}}"#
    ))
    .unwrap();
    let safe = callback_metadata(&payload).unwrap();
    assert_eq!(safe.skipped_fields, [input]);
    assert_eq!(safe.fields.len(), 1);
    assert_eq!(
        safe.fields.get("langsmith_project"),
        Some(&Json::Text("project".to_owned()))
    );
}

fn test_rejected(input: &str, expected: &str) {
    let payload: Json = serde_json::from_str(input).unwrap();
    let error = callback_metadata(&payload).unwrap_err();
    match expected {
        "environment" => assert!(matches!(
            error,
            CallbackMetadataError::EnvironmentReference { .. }
        )),
        "trusted" => assert!(matches!(error, CallbackMetadataError::TrustedField { .. })),
        _ => panic!("unknown expected failure"),
    }
}

fn test_invalid_request(input: &str, _expected: &str) {
    let body = format!(r#"{{"messages":[],{}"#, input.strip_prefix('{').unwrap());
    let request = ChatRequest::parse(body.as_bytes()).unwrap();
    let result = decide_request(
        &request,
        Settings::default(),
        PrivacyContext::default(),
        true,
    );
    let error = result.unwrap_err();
    assert!(matches!(
        error,
        CallbackMetadataError::Json { .. } | CallbackMetadataError::EnvironmentReference { .. }
    ));
}

fn test_forwarding(input: &str, expected: &str) {
    let parent = serde_json::from_str(r#"{"user_api_key_auth":{"budget_reservation":"private","team":"a"},"user_api_key_budget_reservation":"private","route":"r"}"#).unwrap();
    let mode = if input == "attached" {
        Forwarding::Attached
    } else {
        Forwarding::Detached
    };
    let forwarded = forward(&parent, Origin::AutorouterClassifier, mode).unwrap();
    assert_eq!(
        Json::Object(forwarded),
        serde_json::from_str::<Json>(expected).unwrap()
    );
    assert!(parent.contains_key("user_api_key_budget_reservation"));
}

fn test_stream(_input: &str, _expected: &str) {
    let chunk: Chunk = serde_json::from_str(r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"private","tool_calls":[{"index":0,"function":{"name":"tool","arguments":"private"}}],"function_call":{"name":"old","arguments":"private"}}}]}"#).unwrap();
    let original = serde_json::to_string(&chunk).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (private, _) = test_snapshot(
        &chunk,
        &Json::Null,
        Decision {
            content: true,
            key_info: false,
        },
        &redactor,
    )
    .unwrap();
    let serialized = serde_json::to_string(&private).unwrap();
    assert!(!serialized.contains("private"));
    assert!(serialized.contains("tool"));
    assert!(serialized.contains("redacted-by-litellm"));
    assert_eq!(serde_json::to_string(&chunk).unwrap(), original);
}

fn test_response(_input: &str, _expected: &str) {
    let answer: ChatResponse = serde_json::from_str(r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":null,"audio":{"data":"private"},"annotations":["private"],"tool_calls":[{"id":"t","type":"function","function":{"name":"tool","arguments":"private"}}]},"logprobs":{"content":[{"token":"private"}]}}]}"#).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (private, _) = test_snapshot(
        &answer,
        &Json::Null,
        Decision {
            content: true,
            key_info: false,
        },
        &redactor,
    )
    .unwrap();
    let serialized = serde_json::to_string(&private).unwrap();
    assert!(!serialized.contains("private"));
    assert!(serialized.contains("\"content\":null"));
    assert!(serde_json::to_string(&answer).unwrap().contains("private"));
}

fn test_provider_metadata(_input: &str, _expected: &str) {
    let payload = serde_json::from_str(r#"{"served_output_texts":["private"],"vertex_ai_grounding_metadata":["private"],"_hidden_params":{"vertex_ai_grounding_metadata":["private"],"preserved":true},"litellm_params":{"proxy_server_request":{"classifier_input":"private","originating_request_masked":"private"}}}"#).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let private = super::snapshot::callback_snapshot(
        payload,
        Decision {
            content: true,
            key_info: false,
        },
        &redactor,
    );
    assert_eq!(private, serde_json::from_str::<Json>(r#"{"vertex_ai_grounding_metadata":[],"_hidden_params":{"preserved":true},"litellm_params":{"proxy_server_request":{}}}"#).unwrap());
}

fn test_environment(input: &str, expected: &str) {
    let result = validate_environment(input);
    assert_eq!(result.is_ok().to_string(), expected);
}

fn test_span_scope(input: &str, expected: &str) {
    let result = serde_json::from_str::<SpanScope>(&format!("\"{input}\""));
    assert_eq!(result.is_ok().to_string(), expected);
}

fn test_trusted_overlay(_input: &str, _expected: &str) {
    let trusted = TrustedCallbacks {
        values: IndexMap::from([
            (
                "newrelic_api_key".to_owned(),
                SecretString::from("configured-secret"),
            ),
            ("newrelic_region".to_owned(), SecretString::from("eu")),
            ("gcs_bucket_name".to_owned(), SecretString::from("bucket")),
        ]),
    };
    let mut fields = IndexMap::new();
    trusted.overlay(&mut fields);
    assert_eq!(fields.get("newrelic_region").unwrap().expose_secret(), "eu");
    assert_eq!(fields.len(), 2);
    assert_eq!(
        fields.get("newrelic_api_key").unwrap().expose_secret(),
        "configured-secret"
    );
    assert!(!format!("{fields:?}").contains("configured-secret"));
    assert_eq!(
        trusted
            .values
            .get("gcs_bucket_name")
            .unwrap()
            .expose_secret(),
        "bucket"
    );
}

fn test_billing(input: &str, expected: &str) {
    let origin = (input == "poll").then_some(Origin::BackgroundResponseCostPoll);
    let call = if matches!(input, "poll" | "background") {
        "get_responses"
    } else {
        input
    };
    let free = is_unbilled(call_category(Some(call)), origin, input == "background");
    assert_eq!(free.to_string(), expected);
}

fn test_session(_input: &str, _expected: &str) {
    let ids: Session = serde_json::from_str(
        r#"{"litellm_session_id":"session","litellm_trace_id":"trace","unrelated":true}"#,
    )
    .unwrap();
    assert_eq!(ids.litellm_session_id.as_deref(), Some("session"));
    assert_eq!(ids.litellm_trace_id.as_deref(), Some("trace"));
    assert_eq!(serde_json::to_string(&Session::default()).unwrap(), "{}");
}

fn test_child(_input: &str, _expected: &str) {
    let inherited = PrivacyContext {
        message_logging_disabled: true,
    };
    assert!(inherited.message_logging_disabled);
    let request: PrivacyRequest =
        serde_json::from_str(r#"{"turn_off_message_logging":false}"#).unwrap();
    let params = request.dynamic_params(inherited).unwrap();
    assert_eq!(
        params.get("turn_off_message_logging"),
        Some(&Json::Bool(true))
    );
    let decision = PrivacyRequest::default()
        .decide(Settings::default(), inherited, true)
        .unwrap();
    assert!(decision.content);
    assert!(!PrivacyContext::default().message_logging_disabled);
}

fn test_credentials(_input: &str, _expected: &str) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let payload = serde_json::from_str(r#"{"api_key":42,"authorization":{"nested":"private"},"password":false,"secret":null,"note":"sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","safe":[{"token":"private"}]}"#).unwrap();
    let safe = super::snapshot::callback_snapshot(
        payload,
        Decision {
            content: false,
            key_info: false,
        },
        &redactor,
    );
    let expected = serde_json::from_str::<Json>(r#"{"api_key":"REDACTED","authorization":"REDACTED","password":"REDACTED","secret":null,"note":"REDACTED","safe":[{"token":"REDACTED"}]}"#).unwrap();
    assert_eq!(safe, expected);
}

#[rstest]
fn replays_the_privacy_corpus(
    #[files("../betterlite-openai/fuzz/corpus/chat_request/logging-privacy-*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(request) = ChatRequest::parse(&bytes) {
        let _result = decide_request(
            &request,
            Settings::default(),
            PrivacyContext::default(),
            true,
        );
    }
}

fn test_unknown_result(_input: &str, _expected: &str) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let payload = serde_json::from_str(r#"{"response":"private"}"#).unwrap();
    let safe = super::snapshot::callback_snapshot(
        payload,
        Decision {
            content: true,
            key_info: false,
        },
        &redactor,
    );
    assert_eq!(safe, serde_json::from_str::<Json>(r#"{"messages":[{"role":"user","content":"redacted-by-litellm"}],"prompt":"","input":"","response":{"text":"redacted-by-litellm"}}"#).unwrap());
}

fn test_deep_credentials(_input: &str, _expected: &str) {
    let payload = (0_usize..105).fold(Json::Text("deep-private".to_owned()), |child, _depth| {
        Json::List(vec![child])
    });
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let safe = super::snapshot::callback_snapshot(
        payload,
        Decision {
            content: false,
            key_info: false,
        },
        &redactor,
    );
    assert!(
        !serde_json::to_string(&safe)
            .unwrap()
            .contains("deep-private")
    );
}

fn test_empty_forward(_input: &str, _expected: &str) {
    let empty = IndexMap::new();
    assert!(
        forward(&empty, Origin::AutorouterClassifier, Forwarding::Attached)
            .unwrap()
            .is_empty()
    );
    let detached = forward(&empty, Origin::ShadowEvalJudge, Forwarding::Detached).unwrap();
    assert_eq!(
        detached.get("internal_call_origin"),
        Some(&Json::Text("shadow_eval_judge".to_owned()))
    );
}

fn test_metadata_debug(_input: &str, _expected: &str) {
    let payload = serde_json::from_str(r#"{"langsmith_api_key":"private-value"}"#).unwrap();
    let metadata = callback_metadata(&payload).unwrap();
    let debug = format!("{metadata:?}");
    assert!(debug.contains("langsmith_api_key"));
    assert!(!debug.contains("private-value"));
}

fn test_key_policy(input: &str, expected: &str) {
    let request: PrivacyRequest = serde_json::from_str(input).unwrap();
    let settings = Settings {
        turn_off_message_logging: false,
        redact_user_api_key_info: expected == "global",
    };
    let decision = request
        .decide(settings, PrivacyContext::default(), true)
        .unwrap();
    assert!(decision.key_info);
}

fn test_dynamic_params(input: &str, expected: &str) {
    let request: PrivacyRequest = serde_json::from_str(input).unwrap();
    let params = request.dynamic_params(PrivacyContext::default()).unwrap();
    assert_eq!(
        Json::Object(params),
        serde_json::from_str::<Json>(expected).unwrap()
    );
}

fn test_dynamic_error(input: &str, expected: &str) {
    let request: PrivacyRequest = serde_json::from_str(input).unwrap();
    let error = request
        .dynamic_params(PrivacyContext::default())
        .unwrap_err();
    let CallbackMetadataError::EnvironmentReference { field } = error else {
        panic!("expected environment reference rejection")
    };
    assert_eq!(field, expected);
}

fn test_callback_environment(input: &str, expected: &str) {
    let payload: Json = serde_json::from_str(input).unwrap();
    let result = callback_metadata(&payload);
    if expected == "valid" {
        assert_eq!(Json::Object(result.unwrap().fields), payload);
    } else {
        let CallbackMetadataError::InvalidField { field } = result.unwrap_err() else {
            panic!("expected invalid environment")
        };
        assert_eq!(field, "langfuse_environment");
    }
}

fn test_wrapped_result(input: &str, expected: &str) {
    let payload = format!(r#"{{"response":{input}}}"#);
    let snapshot = format!(
        r#"{{"messages":[{{"role":"user","content":"redacted-by-litellm"}}],"prompt":"","input":"","response":{expected}}}"#
    );
    test_snapshot_shape(&payload, &snapshot);
}

fn test_snapshot_shape(input: &str, expected: &str) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let safe = super::snapshot::callback_snapshot(
        serde_json::from_str(input).unwrap(),
        Decision {
            content: true,
            key_info: false,
        },
        &redactor,
    );
    assert_eq!(safe, serde_json::from_str::<Json>(expected).unwrap());
}

fn test_depth_boundary(input: &str, expected: &str) {
    let depth: usize = input.parse().unwrap();
    let payload = (0..depth).fold(Json::Text("public".to_owned()), |child, _depth| {
        Json::List(vec![child])
    });
    let expected_payload = (0_usize..100).fold(Json::Text(expected.to_owned()), |child, _depth| {
        Json::List(vec![child])
    });
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let safe = super::snapshot::callback_snapshot(
        payload,
        Decision {
            content: false,
            key_info: false,
        },
        &redactor,
    );
    assert_eq!(safe, expected_payload);
}

fn test_snapshot<P>(
    source: &P,
    metadata: &Json,
    decision: Decision,
    redactor: &Redactor,
) -> Result<(Json, Json), CallbackMetadataError>
where
    P: Payload,
{
    let payload = source.snapshot(decision.content, redactor).map_err(
        |SnapshotError::Json { source: error }| CallbackMetadataError::Json { source: error },
    )?;
    let mut safe_metadata = redact_user_api_key_info(metadata, decision.key_info);
    super::credentials::scrub(&mut safe_metadata, redactor);
    Ok((
        serde_json::from_slice(&serde_json::to_vec(&payload)?)?,
        safe_metadata,
    ))
}

fn test_typed_metadata(_input: &str, _expected: &str) {
    let request = ChatRequest::parse(br#"{"messages":[{"role":"user","content":"private"}],"metadata":{"input":"campaign-7","text":"tag","api_key":"private"}}"#).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let private = request.snapshot(true, &redactor).unwrap();
    let metadata: Json =
        serde_json::from_slice(&serde_json::to_vec(&private.passthrough).unwrap()).unwrap();
    assert_eq!(
        metadata,
        serde_json::from_str::<Json>(
            r#"{"metadata":{"input":"campaign-7","text":"tag","api_key":"REDACTED"}}"#
        )
        .unwrap()
    );
    assert!(
        !serde_json::to_string(&private.messages)
            .unwrap()
            .contains("private")
    );
    assert!(
        serde_json::to_string(&request.messages)
            .unwrap()
            .contains("private")
    );
}
