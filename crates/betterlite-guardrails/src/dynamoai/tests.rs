#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::routing::post;
use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::dynamoai_moderation::{
    Configured, DEFAULT_API_BASE, DEFAULT_GUARDRAIL_TIMEOUT, Raw, Settings,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::decisions::{
    GuardrailStatus, ViolationDetails, create_error_message, determine_status, process_response,
};
use super::error::DynamoAiError;
use super::guardrail::DynamoAiGuardrail;
use super::hooks::{PostCallOutcome, RequestData, ResponseChoice, ResponseMessage};
use super::wire::{DynamoAiRequest, DynamoAiResponse};

type Server = JoinSet<Result<(), io::Error>>;

fn configured(api_base: &str) -> Configured {
    Configured {
        guardrail_name: "dynamoai".to_owned(),
        api_key: SecretString::from("oracle-key"),
        api_base: api_base.to_owned(),
        policy_ids: Vec::new(),
        model_id: String::new(),
        timeout: Duration::from_secs(30),
    }
}

fn guardrail(api_base: &str) -> DynamoAiGuardrail {
    DynamoAiGuardrail {
        settings: configured(api_base),
        client: reqwest::Client::default(),
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn user_message() -> GuardrailValue {
    object(&[("role", text("user")), ("content", text("hello"))])
}

fn parse(body: &str) -> DynamoAiResponse {
    serde_json::from_str(body).expect("the test answer is a DynamoAI response")
}

async fn answering(status: StatusCode, body: String, delay: Duration) -> (String, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let answer = move || {
        let answer_body = body.clone();
        async move {
            sleep(delay).await;
            (status, answer_body)
        }
    };
    let router = Router::new().route("/v1/moderation/analyze/", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

async fn capturing(body: String) -> (String, Arc<Mutex<Vec<u8>>>, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let captured_for_handler = Arc::clone(&captured);
    let answer = move |request: Bytes| {
        let captured_for_body = Arc::clone(&captured_for_handler);
        let answer_body = body.clone();
        async move {
            *captured_for_body.lock().unwrap() = request.to_vec();
            (StatusCode::OK, answer_body)
        }
    };
    let router = Router::new().route("/v1/moderation/analyze/", post(answer));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, captured, server)
}

fn captured_body(captured: &Arc<Mutex<Vec<u8>>>) -> DynamoAiRequest {
    let bytes = captured.lock().unwrap().clone();
    serde_json::from_slice(&bytes).expect("the guardrail sends a JSON body")
}

#[test]
fn dynamoai_guardrail_prepare_headers_uses_a_bearer_token() {
    let headers = guardrail(DEFAULT_API_BASE).prepare_headers().unwrap();
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer oracle-key"
    );
    assert_eq!(
        headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

#[test]
fn dynamoai_guardrail_prepare_headers_rejects_a_key_that_is_not_a_header() {
    let mut instance = guardrail(DEFAULT_API_BASE);
    instance.settings.api_key = SecretString::from("bad\nkey");
    assert!(matches!(
        instance.prepare_headers(),
        Err(DynamoAiError::InvalidHeader { .. })
    ));
}

#[test]
fn dynamoai_guardrail_prepare_payload_omits_empty_policy_ids_and_model_id() {
    let payload = guardrail(DEFAULT_API_BASE).prepare_payload(&[user_message()]);
    assert_eq!(payload.messages, vec![user_message()]);
    assert_eq!(payload.policy_ids, None);
    assert_eq!(payload.model_id, None);
}

#[test]
fn dynamoai_guardrail_prepare_payload_carries_policy_ids_and_model_id() {
    let mut instance = guardrail(DEFAULT_API_BASE);
    instance.settings.policy_ids = vec!["policy-1".to_owned(), "policy-2".to_owned()];
    instance.settings.model_id = "gpt-4o".to_owned();
    let payload = instance.prepare_payload(&[user_message()]);
    assert_eq!(
        payload.policy_ids,
        Some(vec!["policy-1".to_owned(), "policy-2".to_owned()])
    );
    assert_eq!(payload.model_id.as_deref(), Some("gpt-4o"));
}

#[test]
fn dynamoai_guardrail_default_api_base_is_https() {
    assert_eq!(DEFAULT_API_BASE, "https://api.dynamo.ai");
}

#[rstest]
#[case::none("{}", GuardrailStatus::Success)]
#[case::warn(r#"{"finalAction":"WARN"}"#, GuardrailStatus::Success)]
#[case::redact(r#"{"finalAction":"REDACT"}"#, GuardrailStatus::Success)]
#[case::sanitize(r#"{"finalAction":"SANITIZE"}"#, GuardrailStatus::Success)]
#[case::block(r#"{"finalAction":"BLOCK"}"#, GuardrailStatus::Intervened)]
#[case::error(
    r#"{"finalAction":"NONE","error":"upstream exploded"}"#,
    GuardrailStatus::FailedToRespond
)]
fn dynamoai_guardrail_status_classifies_the_final_action(
    #[case] body: &str,
    #[case] expected: GuardrailStatus,
) {
    assert_eq!(determine_status(&parse(body)), expected);
}

#[test]
fn dynamoai_guardrail_applied_policies_extract_every_detail() {
    let response = parse(
        r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"id":"p-1","name":"Toxicity","method":"TOXICITY","description":"no insults"},"outputs":{"action":"BLOCK","message":"blocked text"}}]}"#,
    );
    let processed = process_response(&response);
    assert_eq!(processed.violations_detected, vec!["Toxicity".to_owned()]);
    assert_eq!(
        processed.violation_details.get("Toxicity"),
        Some(&ViolationDetails {
            policy_id: Some("p-1".to_owned()),
            action: "BLOCK".to_owned(),
            method: Some("TOXICITY".to_owned()),
            description: Some("no insults".to_owned()),
            message: Some("blocked text".to_owned()),
        })
    );
}

#[test]
fn dynamoai_guardrail_policy_action_prefers_the_applied_policy_field() {
    let response = parse(
        r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"name":"Outer"},"outputs":{"action":"BLOCK"},"action":"WARN"},{"policy":{"name":"Inner"},"outputs":{"action":"BLOCK"}}]}"#,
    );
    let processed = process_response(&response);
    assert_eq!(processed.violations_detected, vec!["Inner".to_owned()]);
}

#[test]
fn dynamoai_guardrail_block_without_a_block_policy_has_no_violations() {
    let response = parse(
        r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"name":"Warned"},"outputs":{"action":"WARN"}}]}"#,
    );
    let processed = process_response(&response);
    assert!(processed.violations_detected.is_empty());
    assert!(processed.violation_details.is_empty());
}

#[rstest]
#[case::warn("WARN")]
#[case::redact("REDACT")]
#[case::sanitize("SANITIZE")]
#[case::none("NONE")]
fn dynamoai_guardrail_non_block_actions_report_no_violations(#[case] action: &str) {
    let body = format!(
        r#"{{"finalAction":"{action}","appliedPolicies":[{{"policy":{{"name":"Strict"}},"outputs":{{"action":"BLOCK"}}}}]}}"#
    );
    assert!(
        process_response(&parse(&body))
            .violations_detected
            .is_empty()
    );
}

#[test]
fn dynamoai_guardrail_unknown_policy_name_falls_back_to_unknown() {
    let response =
        parse(r#"{"finalAction":"BLOCK","appliedPolicies":[{"outputs":{"action":"BLOCK"}}]}"#);
    assert_eq!(
        process_response(&response).violations_detected,
        vec!["unknown".to_owned()]
    );
}

#[rstest]
#[case::success(GuardrailStatus::Success, "success")]
#[case::intervened(GuardrailStatus::Intervened, "guardrail_intervened")]
#[case::failed(GuardrailStatus::FailedToRespond, "guardrail_failed_to_respond")]
fn dynamoai_guardrail_status_names_match_litellm(
    #[case] status: GuardrailStatus,
    #[case] expected: &str,
) {
    assert_eq!(status.as_str(), expected);
}

#[test]
fn dynamoai_guardrail_error_message_skips_empty_details() {
    let response = parse(
        r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"name":"toxicity","description":""},"outputs":{"action":"BLOCK"}}]}"#,
    );
    let message = create_error_message(&process_response(&response));
    assert!(message.ends_with("  Action: BLOCK"));
    assert!(!message.contains("Description"));
}

#[test]
fn dynamoai_guardrail_error_message_lists_the_violation_details() {
    let response = parse(
        r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"id":"p-1","name":"toxicity","method":"TOXICITY","description":"no insults"},"outputs":{"action":"BLOCK","message":"blocked text"}}]}"#,
    );
    let message = create_error_message(&process_response(&response));
    assert!(message.starts_with("the DynamoAI guardrail blocked the request: 1 violation(s)"));
    assert!(message.contains("- TOXICITY:"));
    assert!(message.contains("  Action: BLOCK\n"));
    assert!(message.contains("  Method: TOXICITY\n"));
    assert!(message.contains("  Description: no insults\n"));
    assert!(message.contains("  Message: blocked text\n"));
    assert!(message.ends_with("  Policy ID: p-1"));
}

#[tokio::test]
async fn dynamoai_guardrail_call_sends_the_path_and_the_raw_messages() {
    let extra = object(&[
        ("role", text("user")),
        ("content", text("hello")),
        ("name", text("alice")),
    ]);
    let (base, captured, _server) = capturing(r#"{"finalAction":"NONE"}"#.to_owned()).await;
    guardrail(&base)
        .call_moderation(std::slice::from_ref(&extra))
        .await
        .unwrap();
    assert_eq!(captured_body(&captured).messages, vec![extra]);
}

#[tokio::test]
async fn dynamoai_guardrail_call_sends_policy_ids_and_model_id() {
    let mut instance = guardrail("http://127.0.0.1:1");
    instance.settings.policy_ids = vec!["policy-1".to_owned()];
    instance.settings.model_id = "model-1".to_owned();
    let (base, captured, _server) = capturing(r#"{"finalAction":"NONE"}"#.to_owned()).await;
    instance.settings.api_base = base;
    instance.call_moderation(&[user_message()]).await.unwrap();
    let body = captured_body(&captured);
    assert_eq!(body.policy_ids, Some(vec!["policy-1".to_owned()]));
    assert_eq!(body.model_id, Some("model-1".to_owned()));
}

#[tokio::test]
async fn dynamoai_guardrail_reads_an_answer_larger_than_a_few_kilobytes() {
    let padding = "x".repeat(10_000);
    let body = format!(r#"{{"finalAction":"NONE","padding":"{padding}"}}"#);
    let (base, _server) = answering(StatusCode::OK, body, Duration::ZERO).await;
    guardrail(&base)
        .call_moderation(&[user_message()])
        .await
        .unwrap();
}

#[tokio::test]
async fn dynamoai_guardrail_an_answer_over_a_mebibyte_is_rejected() {
    let padding = "x".repeat(1024 * 1024);
    let body = format!(r#"{{"finalAction":"NONE","padding":"{padding}"}}"#);
    let (base, _server) = answering(StatusCode::OK, body, Duration::ZERO).await;
    let result = guardrail(&base).call_moderation(&[user_message()]).await;
    assert!(matches!(
        result,
        Err(DynamoAiError::ResponseTooLarge { limit: 0x0010_0000 })
    ));
}

#[tokio::test]
async fn dynamoai_guardrail_a_stalled_answer_times_out() {
    let (base, _server) = answering(
        StatusCode::OK,
        r#"{"finalAction":"NONE"}"#.to_owned(),
        Duration::from_millis(500),
    )
    .await;
    let mut instance = guardrail(&base);
    instance.settings.timeout = Duration::from_millis(50);
    let result = instance.call_moderation(&[user_message()]).await;
    assert!(matches!(result, Err(DynamoAiError::Timeout { .. })));
}

#[tokio::test]
async fn dynamoai_guardrail_http_error_is_reported() {
    let (base, _server) = answering(
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}".to_owned(),
        Duration::ZERO,
    )
    .await;
    let result = guardrail(&base).call_moderation(&[user_message()]).await;
    assert!(matches!(result, Err(DynamoAiError::HttpStatus { .. })));
}

#[tokio::test]
async fn dynamoai_guardrail_non_object_response_is_a_json_error() {
    let (base, _server) = answering(
        StatusCode::OK,
        r#"["not","an","object"]"#.to_owned(),
        Duration::ZERO,
    )
    .await;
    let result = guardrail(&base).call_moderation(&[user_message()]).await;
    assert!(matches!(result, Err(DynamoAiError::ResponseJson { .. })));
}

#[tokio::test]
async fn dynamoai_guardrail_pre_call_blocks_on_a_block_policy() {
    let body = r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"name":"Toxicity"},"outputs":{"action":"BLOCK","message":"no"}}]}"#;
    let (base, _server) = answering(StatusCode::OK, body.to_owned(), Duration::ZERO).await;
    let result = guardrail(&base)
        .async_pre_call_hook(&RequestData {
            messages: vec![user_message()],
        })
        .await;
    assert!(matches!(result, Err(DynamoAiError::Blocked { .. })));
}

#[tokio::test]
async fn dynamoai_guardrail_pre_call_allows_a_block_action_without_a_block_policy() {
    let body = r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"name":"Warned"},"outputs":{"action":"WARN"}}]}"#;
    let (base, _server) = answering(StatusCode::OK, body.to_owned(), Duration::ZERO).await;
    let result = guardrail(&base)
        .async_pre_call_hook(&RequestData {
            messages: vec![user_message()],
        })
        .await;
    assert!(matches!(result, Ok(GuardrailStatus::Intervened)));
}

#[tokio::test]
async fn dynamoai_guardrail_during_call_blocks_on_a_block_policy() {
    let body = r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"name":"Toxicity"},"outputs":{"action":"BLOCK"}}]}"#;
    let (base, captured, _server) = capturing(body.to_owned()).await;
    let result = guardrail(&base)
        .async_moderation_hook(&[user_message()])
        .await;
    assert!(matches!(result, Err(DynamoAiError::Blocked { .. })));
    assert_eq!(captured_body(&captured).messages, vec![user_message()]);
}

#[tokio::test]
async fn dynamoai_guardrail_hooks_skip_a_call_without_messages() {
    let instance = guardrail("http://127.0.0.1:1");
    let empty = RequestData {
        messages: Vec::new(),
    };
    assert!(matches!(
        instance.async_pre_call_hook(&empty).await,
        Ok(GuardrailStatus::Success)
    ));
    assert!(matches!(
        instance.async_moderation_hook(&[]).await,
        Ok(GuardrailStatus::Success)
    ));
}

#[tokio::test]
async fn dynamoai_guardrail_post_call_forwards_role_and_content() {
    let (base, captured, _server) = capturing(r#"{"finalAction":"NONE"}"#.to_owned()).await;
    let choices = vec![ResponseChoice {
        message: ResponseMessage {
            role: Some("assistant".to_owned()),
            content: Some(text("an answer")),
        },
    }];
    let result = guardrail(&base)
        .async_post_call_success_hook(&choices)
        .await;
    assert!(matches!(
        result,
        Ok(PostCallOutcome::Inspected(GuardrailStatus::Success))
    ));
    assert_eq!(
        captured_body(&captured).messages,
        vec![object(&[
            ("role", text("assistant")),
            ("content", text("an answer")),
        ])]
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(String::new()))]
#[tokio::test]
async fn dynamoai_guardrail_post_call_defaults_the_role_to_assistant(#[case] role: Option<String>) {
    let (base, captured, _server) = capturing(r#"{"finalAction":"NONE"}"#.to_owned()).await;
    let choices = vec![ResponseChoice {
        message: ResponseMessage {
            role,
            content: Some(text("an answer")),
        },
    }];
    guardrail(&base)
        .async_post_call_success_hook(&choices)
        .await
        .unwrap();
    assert_eq!(
        captured_body(&captured).messages,
        vec![object(&[
            ("role", text("assistant")),
            ("content", text("an answer")),
        ])]
    );
}

#[tokio::test]
async fn dynamoai_guardrail_post_call_skips_non_string_content_silently() {
    let choices = vec![
        ResponseChoice {
            message: ResponseMessage {
                role: Some("assistant".to_owned()),
                content: Some(GuardrailValue::Array(vec![text("part")])),
            },
        },
        ResponseChoice {
            message: ResponseMessage {
                role: Some("assistant".to_owned()),
                content: None,
            },
        },
        ResponseChoice {
            message: ResponseMessage {
                role: Some("assistant".to_owned()),
                content: Some(text("")),
            },
        },
    ];
    let result = guardrail("http://127.0.0.1:1")
        .async_post_call_success_hook(&choices)
        .await;
    assert!(matches!(result, Ok(PostCallOutcome::NoText)));
}

#[tokio::test]
async fn dynamoai_guardrail_post_call_blocks_on_a_block_policy() {
    let body = r#"{"finalAction":"BLOCK","appliedPolicies":[{"policy":{"name":"Toxicity"},"outputs":{"action":"BLOCK"}}]}"#;
    let (base, _server) = answering(StatusCode::OK, body.to_owned(), Duration::ZERO).await;
    let choices = vec![ResponseChoice {
        message: ResponseMessage {
            role: Some("assistant".to_owned()),
            content: Some(text("bad answer")),
        },
    }];
    let result = guardrail(&base)
        .async_post_call_success_hook(&choices)
        .await;
    assert!(matches!(result, Err(DynamoAiError::Blocked { .. })));
}

fn entry(guardrail: &str, params: &str) -> String {
    format!(
        r#"{{"guardrails":[{{"guardrail_name":"{guardrail}","litellm_params":{{"guardrail":"{guardrail}"{params}}}}}]}}"#
    )
}

fn convert(document: &str, env: &BTreeMap<String, String>) -> Result<Settings, ConfigError> {
    let raw: Raw =
        serde_saphyr::from_str(document).map_err(|source| ConfigError::Yaml { source })?;
    raw.convert(env)
}

fn env(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn dynamoai_config_reads_the_api_key_from_the_environment() {
    let settings = convert(
        &entry("dynamoai", ""),
        &env(&[("DYNAMOAI_API_KEY", "env-key")]),
    )
    .unwrap();
    assert_eq!(settings.guardrails[0].api_key.expose_secret(), "env-key");
    assert_eq!(settings.guardrails[0].api_base, DEFAULT_API_BASE);
    assert!(settings.guardrails[0].policy_ids.is_empty());
    assert_eq!(settings.guardrails[0].model_id, "");
}

#[test]
fn dynamoai_config_ignores_guardrails_of_other_providers() {
    let document =
        r#"{"guardrails":[{"guardrail_name":"other","litellm_params":{"guardrail":"pangea"}}]}"#;
    let settings = convert(document, &BTreeMap::new()).unwrap();
    assert!(settings.guardrails.is_empty());
}

#[test]
fn dynamoai_config_missing_api_key_names_the_variable() {
    let error = convert(&entry("dynamoai", ""), &BTreeMap::new()).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, .. } if variable == "DYNAMOAI_API_KEY"
    ));
}

#[test]
fn dynamoai_config_reads_api_base_policy_ids_and_model_id_from_the_environment() {
    let settings = convert(
        &entry("dynamoai", ""),
        &env(&[
            ("DYNAMOAI_API_KEY", "env-key"),
            ("DYNAMOAI_API_BASE", "http://dynamo.test"),
            ("DYNAMOAI_POLICY_IDS", "policy-1,policy-2"),
            ("DYNAMOAI_MODEL_ID", "model-1"),
        ]),
    )
    .unwrap();
    assert_eq!(settings.guardrails[0].api_base, "http://dynamo.test");
    assert_eq!(
        settings.guardrails[0].policy_ids,
        vec!["policy-1".to_owned(), "policy-2".to_owned()]
    );
    assert_eq!(settings.guardrails[0].model_id, "model-1");
}

#[test]
fn dynamoai_config_keeps_spaces_around_environment_policy_ids() {
    let settings = convert(
        &entry("dynamoai", ""),
        &env(&[
            ("DYNAMOAI_API_KEY", "env-key"),
            ("DYNAMOAI_POLICY_IDS", "policy-1, policy-2"),
        ]),
    )
    .unwrap();
    assert_eq!(
        settings.guardrails[0].policy_ids,
        vec!["policy-1".to_owned(), " policy-2".to_owned()]
    );
}

#[test]
fn dynamoai_config_empty_policy_ids_environment_is_an_empty_list() {
    let settings = convert(
        &entry("dynamoai", ""),
        &env(&[
            ("DYNAMOAI_API_KEY", "env-key"),
            ("DYNAMOAI_POLICY_IDS", ""),
            ("DYNAMOAI_MODEL_ID", ""),
        ]),
    )
    .unwrap();
    assert!(settings.guardrails[0].policy_ids.is_empty());
    assert_eq!(settings.guardrails[0].model_id, "");
}

#[test]
fn dynamoai_config_reads_the_timeout_in_seconds() {
    let settings = convert(
        &entry("dynamoai", r#","api_key":"param-key","timeout":2.5"#),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(settings.guardrails[0].timeout, Duration::from_millis(2500));
}

#[test]
fn dynamoai_config_resolves_an_environment_reference_in_the_timeout() {
    let settings = convert(
        &entry(
            "dynamoai",
            r#","api_key":"param-key","timeout":"os.environ/DYNAMOAI_TIMEOUT""#,
        ),
        &env(&[("DYNAMOAI_TIMEOUT", "7.5")]),
    )
    .unwrap();
    assert_eq!(settings.guardrails[0].timeout, Duration::from_millis(7500));
}

#[test]
fn dynamoai_config_defaults_the_timeout_when_absent() {
    let settings = convert(
        &entry("dynamoai", ""),
        &env(&[("DYNAMOAI_API_KEY", "env-key")]),
    )
    .unwrap();
    assert_eq!(settings.guardrails[0].timeout, DEFAULT_GUARDRAIL_TIMEOUT);
    assert_eq!(DEFAULT_GUARDRAIL_TIMEOUT, Duration::from_secs(600));
}

#[test]
fn dynamoai_config_empty_parameters_fall_back_to_the_environment() {
    let settings = convert(
        &entry("dynamoai", r#","api_key":"","api_base":"","model_id":"""#),
        &env(&[
            ("DYNAMOAI_API_KEY", "env-key"),
            ("DYNAMOAI_API_BASE", "http://env.test"),
            ("DYNAMOAI_MODEL_ID", "env-model"),
        ]),
    )
    .unwrap();
    let configured = &settings.guardrails[0];
    assert_eq!(configured.api_key.expose_secret(), "env-key");
    assert_eq!(configured.api_base, "http://env.test");
    assert_eq!(configured.model_id, "env-model");
}

#[test]
fn dynamoai_config_empty_environment_api_base_uses_the_default() {
    let settings = convert(
        &entry("dynamoai", ""),
        &env(&[("DYNAMOAI_API_KEY", "env-key"), ("DYNAMOAI_API_BASE", "")]),
    )
    .unwrap();
    assert_eq!(settings.guardrails[0].api_base, DEFAULT_API_BASE);
}

#[test]
fn dynamoai_config_empty_environment_api_key_is_missing() {
    let error = convert(&entry("dynamoai", ""), &env(&[("DYNAMOAI_API_KEY", "")])).unwrap_err();
    assert!(matches!(error, ConfigError::MissingEnvironment { .. }));
}

#[test]
fn dynamoai_config_rejects_a_negative_timeout() {
    let error = convert(
        &entry("dynamoai", r#","api_key":"param-key","timeout":-1"#),
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::Yaml { .. }));
}

#[test]
fn dynamoai_config_parameters_win_over_the_environment() {
    let settings = convert(
        &entry(
            "dynamoai",
            r#","api_key":"param-key","api_base":"http://param.test","policy_ids":["param-policy"],"model_id":"param-model""#,
        ),
        &env(&[
            ("DYNAMOAI_API_KEY", "env-key"),
            ("DYNAMOAI_API_BASE", "http://env.test"),
            ("DYNAMOAI_POLICY_IDS", "env-policy"),
            ("DYNAMOAI_MODEL_ID", "env-model"),
        ]),
    )
    .unwrap();
    let configured = &settings.guardrails[0];
    assert_eq!(configured.api_key.expose_secret(), "param-key");
    assert_eq!(configured.api_base, "http://param.test");
    assert_eq!(configured.policy_ids, vec!["param-policy".to_owned()]);
    assert_eq!(configured.model_id, "param-model");
}

#[test]
fn dynamoai_config_resolves_an_environment_reference_in_a_parameter() {
    let settings = convert(
        &entry(
            "dynamoai",
            r#","api_key":"os.environ/DYNAMOAI_KEY_FROM_REFERENCE""#,
        ),
        &env(&[("DYNAMOAI_KEY_FROM_REFERENCE", "referenced-key")]),
    )
    .unwrap();
    assert_eq!(
        settings.guardrails[0].api_key.expose_secret(),
        "referenced-key"
    );
}
