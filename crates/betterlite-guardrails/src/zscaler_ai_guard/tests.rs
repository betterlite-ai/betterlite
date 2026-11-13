#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_http::proxy::base_builder;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::error::{ZscalerAiGuardError, ZscalerErrorDetail};
use super::init::ZscalerArgs;
use super::metadata;
use super::printed::{quote_text, repr_text, str_text};
use super::settings::{self, DEFAULT_TIMEOUT_DURATION, Settings};
use super::{
    API_KEY_ENV, DEFAULT_URL, Inputs, POLICY_ID_ENV, SEND_ALIAS_ENV, SEND_TEAM_ID_ENV,
    SEND_USER_ID_ENV, SUPPORTED_EVENT_HOOKS, URL_ENV, ZscalerAiGuard,
};
use crate::generic_guardrail_contracts::GuardrailInputType;

struct Seen {
    path: String,
    headers: HeaderMap,
    body: GuardrailValue,
}

struct Mock {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: tokio::task::JoinSet<std::io::Result<()>>,
}

async fn mock(status: u16, reply: &str) -> Mock {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&recorded);
    let reply = reply.to_owned();
    let answer = move |uri: Uri, headers: HeaderMap, bytes: Bytes| {
        let recorder = Arc::clone(&recorder);
        let reply = reply.clone();
        async move {
            let body =
                serde_json::from_slice::<GuardrailValue>(&bytes).unwrap_or(GuardrailValue::Null);
            recorder.lock().unwrap().push(Seen {
                path: uri.path().to_owned(),
                headers,
                body,
            });
            (
                StatusCode::from_u16(status).unwrap(),
                [("content-type", "application/json")],
                reply,
            )
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = tokio::task::JoinSet::new();
    server.spawn(axum::serve(listener, Router::new().fallback(answer)).into_future());
    Mock {
        base,
        seen: recorded,
        _server: server,
    }
}

fn args() -> ZscalerArgs {
    ZscalerArgs {
        name: "zscaler-ai-guard".to_owned(),
        api_base: None,
        api_key: Some(SecretString::from("oracle-key")),
        policy_id: Some(42),
        send_user_api_key_alias: Some(false),
        send_user_api_key_user_id: Some(false),
        send_user_api_key_team_id: Some(false),
        timeout: Some(2.0),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn guardrail(base: &str) -> ZscalerAiGuard {
    let mut args = args();
    args.api_base = Some(base.to_owned());
    let environment: BTreeMap<String, String> = BTreeMap::new();
    ZscalerAiGuard::new(args, &environment, base_builder().build().unwrap()).unwrap()
}

fn inputs(texts: &[&str]) -> Inputs {
    Inputs {
        texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        rest: BTreeMap::new(),
    }
}

fn request_data(json: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(json).unwrap()
}

fn captured<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&recorded);
    let subscriber = fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || Captured(Arc::clone(&sink)))
        .finish();
    let result = with_default(subscriber, run);
    let bytes = recorded.lock().unwrap().clone();
    (result, String::from_utf8(bytes).unwrap())
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn supported_hooks_are_pre_call_and_post_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [super::CallHook::PreCall, super::CallHook::PostCall]
    );
}

#[test]
fn settings_use_the_argument_url_over_the_environment() {
    let mut args = args();
    args.api_base = Some("https://argument.example".to_owned());
    let resolved =
        Settings::resolve(&args, &environment(&[(URL_ENV, "https://env.example")])).unwrap();
    assert_eq!(resolved.url, "https://argument.example");
}

#[test]
fn settings_fall_back_to_the_url_environment_variable() {
    let resolved =
        Settings::resolve(&args(), &environment(&[(URL_ENV, "https://env.example")])).unwrap();
    assert_eq!(resolved.url, "https://env.example");
}

#[test]
fn settings_use_the_default_url_when_nothing_is_set() {
    let resolved = Settings::resolve(&args(), &environment(&[])).unwrap();
    assert_eq!(resolved.url, DEFAULT_URL);
}

#[test]
fn settings_ignore_an_empty_url_environment_variable() {
    let resolved = Settings::resolve(&args(), &environment(&[(URL_ENV, "")])).unwrap();
    assert_eq!(resolved.url, DEFAULT_URL);
}

#[test]
fn settings_keep_an_explicit_empty_url() {
    let mut args = args();
    args.api_base = Some(String::new());
    let resolved =
        Settings::resolve(&args, &environment(&[(URL_ENV, "https://env.example")])).unwrap();
    assert_eq!(resolved.url, "");
}

#[test]
fn settings_use_the_api_key_argument_over_the_environment() {
    let mut args = args();
    args.api_key = Some(SecretString::from("argument-key"));
    let resolved = Settings::resolve(&args, &environment(&[(API_KEY_ENV, "env-key")])).unwrap();
    assert_eq!(
        resolved.api_key.as_ref().unwrap().expose_secret(),
        "argument-key"
    );
}

#[test]
fn settings_fall_back_to_the_api_key_environment_variable() {
    let mut args = args();
    args.api_key = None;
    let resolved = Settings::resolve(&args, &environment(&[(API_KEY_ENV, "env-key")])).unwrap();
    assert_eq!(
        resolved.api_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
}

#[test]
fn settings_leave_a_missing_api_key_unset() {
    let mut args = args();
    args.api_key = None;
    let resolved = Settings::resolve(&args, &environment(&[(API_KEY_ENV, "")])).unwrap();
    assert!(resolved.api_key.is_none());
}

#[test]
fn settings_use_the_policy_id_argument_over_the_environment() {
    let mut args = args();
    args.policy_id = Some(5);
    let resolved = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "9")])).unwrap();
    assert_eq!(resolved.policy_id, 5);
}

#[test]
fn settings_fall_back_to_the_policy_id_environment_variable() {
    let mut args = args();
    args.policy_id = None;
    let resolved = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "9")])).unwrap();
    assert_eq!(resolved.policy_id, 9);
}

#[test]
fn settings_default_the_policy_id_to_minus_one() {
    let mut args = args();
    args.policy_id = None;
    let resolved = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "")])).unwrap();
    assert_eq!(resolved.policy_id, -1);
}

#[test]
fn settings_refuse_a_non_integer_policy_id_environment_variable() {
    let mut args = args();
    args.policy_id = None;
    let error = Settings::resolve(&args, &environment(&[(POLICY_ID_ENV, "seven")])).unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::PolicyId { .. }));
    assert!(error.to_string().contains(POLICY_ID_ENV));
}

#[rstest]
#[case::argument_true(Some(true), None, true)]
#[case::argument_false_over_env(Some(false), Some("true"), false)]
#[case::env_true(None, Some("TRUE"), true)]
#[case::env_one(None, Some("1"), true)]
#[case::env_yes(None, Some("yes"), true)]
#[case::env_on(None, Some("on"), true)]
#[case::env_padded(None, Some(" true "), true)]
#[case::env_zero(None, Some("0"), false)]
#[case::env_false(None, Some("false"), false)]
#[case::env_empty(None, Some(""), false)]
#[case::env_absent(None, None, false)]
fn send_flags_resolve_from_the_argument_then_the_environment(
    #[case] argument: Option<bool>,
    #[case] environment_value: Option<&str>,
    #[case] expected: bool,
) {
    let mut args = args();
    args.send_user_api_key_alias = argument;
    let environment = environment_value.map_or_else(BTreeMap::new, |value| {
        environment(&[(SEND_ALIAS_ENV, value)])
    });
    let resolved = Settings::resolve(&args, &environment).unwrap();
    assert_eq!(resolved.send.alias, expected);
}

#[test]
fn settings_refuse_an_unparsable_send_flag() {
    let mut args = args();
    args.send_user_api_key_alias = None;
    let error = Settings::resolve(&args, &environment(&[(SEND_ALIAS_ENV, "maybe")])).unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::SendFlag { .. }));
    assert!(error.to_string().contains(SEND_ALIAS_ENV));
}

#[test]
fn a_false_send_flag_argument_beats_a_true_environment_value() {
    let mut args = args();
    args.send_user_api_key_alias = Some(false);
    let resolved = Settings::resolve(&args, &environment(&[(SEND_ALIAS_ENV, "true")])).unwrap();
    assert!(!resolved.send.alias);
}

#[test]
fn user_id_and_team_id_flags_read_their_own_environment_variables() {
    let mut args = args();
    args.send_user_api_key_user_id = None;
    args.send_user_api_key_team_id = None;
    let environment = environment(&[(SEND_USER_ID_ENV, "true"), (SEND_TEAM_ID_ENV, "1")]);
    let resolved = Settings::resolve(&args, &environment).unwrap();
    assert!(resolved.send.user_id);
    assert!(resolved.send.team_id);
}

#[test]
fn settings_keep_a_positive_timeout() {
    let mut args = args();
    args.timeout = Some(1.5);
    let resolved = Settings::resolve(&args, &environment(&[])).unwrap();
    assert_eq!(resolved.timeout, Duration::from_millis(1500));
}

#[test]
fn settings_default_the_timeout_when_it_is_absent() {
    let mut args = args();
    args.timeout = None;
    let resolved = Settings::resolve(&args, &environment(&[])).unwrap();
    assert_eq!(resolved.timeout, DEFAULT_TIMEOUT_DURATION);
}

#[test]
fn a_zero_timeout_falls_back_to_five_seconds() {
    let (timeout, _written) = captured(|| settings::resolve_timeout(Some(0.0)).unwrap());
    assert_eq!(timeout, DEFAULT_TIMEOUT_DURATION);
}

#[rstest]
#[case::zero(0.0)]
#[case::negative(-1.5)]
fn non_positive_timeouts_fall_back_and_name_the_value_and_default(#[case] value: f64) {
    let (timeout, written) = captured(|| settings::resolve_timeout(Some(value)).unwrap());
    assert_eq!(timeout, DEFAULT_TIMEOUT_DURATION);
    assert!(written.contains(&format!("timeout {value}")));
    assert!(written.contains("using 5 seconds"));
}

#[test]
fn settings_refuse_an_unrepresentable_timeout() {
    let error = settings::resolve_timeout(Some(f64::INFINITY)).unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::Timeout { .. }));
}

#[test]
fn update_params_takes_the_new_timeout() {
    let mut guardrail = guardrail("http://127.0.0.1:1");
    let params = decode_litellm_params(
        br#"{"guardrail": "zscaler_ai_guard", "mode": "pre_call", "timeout": 9.5}"#,
    )
    .unwrap();
    guardrail.update_params(&params).unwrap();
    assert_eq!(guardrail.settings.timeout, Duration::from_millis(9500));
}

#[test]
fn update_params_falls_back_on_a_non_positive_timeout() {
    let mut guardrail = guardrail("http://127.0.0.1:1");
    let params = decode_litellm_params(
        br#"{"guardrail": "zscaler_ai_guard", "mode": "pre_call", "timeout": -2.0}"#,
    )
    .unwrap();
    let (_result, written) = captured(|| guardrail.update_params(&params).unwrap());
    assert_eq!(guardrail.settings.timeout, DEFAULT_TIMEOUT_DURATION);
    assert!(written.contains("-2"));
}

#[test]
fn policy_id_comes_from_request_metadata_first() {
    let data = request_data(
        r#"{
            "metadata": {
                "zguard_policy_id": 1,
                "user_api_key_metadata": {"zguard_policy_id": 2},
                "team_metadata": {"zguard_policy_id": 3}
            }
        }"#,
    );
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(1));
}

#[test]
fn policy_id_comes_from_the_user_key_metadata_next() {
    let data = request_data(
        r#"{
            "metadata": {
                "user_api_key_metadata": {"zguard_policy_id": 2},
                "team_metadata": {"zguard_policy_id": 3}
            }
        }"#,
    );
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(2));
}

#[test]
fn policy_id_comes_from_the_team_metadata_next() {
    let data = request_data(r#"{"metadata": {"team_metadata": {"zguard_policy_id": 3}}}"#);
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(3));
}

#[test]
fn a_null_request_policy_id_still_wins_over_the_configured_id() {
    let data = request_data(
        r#"{"metadata": {"zguard_policy_id": null, "team_metadata": {"zguard_policy_id": 3}}}"#,
    );
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::Null);
}

#[test]
fn policy_id_defaults_to_the_configured_id() {
    let data = request_data(r#"{"metadata": {}}"#);
    let selected = metadata::selected_policy_id(&data, 4).unwrap();
    assert_eq!(selected, GuardrailValue::SignedInteger(4));
}

#[test]
fn metadata_values_prefer_litellm_metadata_over_metadata() {
    let data = request_data(
        r#"{
            "litellm_metadata": {"user_api_key_alias": "from-litellm"},
            "metadata": {"user_api_key_alias": "from-metadata"}
        }"#,
    );
    let value = metadata::header_value(&data, "user_api_key_alias").unwrap();
    assert_eq!(value, "from-litellm");
}

#[test]
fn the_alias_falls_back_to_the_key_alias_in_litellm_metadata() {
    let data = request_data(r#"{"litellm_metadata": {"user_api_key_key_alias": "key-alias"}}"#);
    let value = metadata::header_value(&data, "user_api_key_alias").unwrap();
    assert_eq!(value, "key-alias");
}

#[test]
fn metadata_values_are_trimmed() {
    let data = request_data(r#"{"metadata": {"user_api_key_user_id": "  user-1  "}}"#);
    let value = metadata::header_value(&data, "user_api_key_user_id").unwrap();
    assert_eq!(value, "user-1");
}

#[test]
fn metadata_header_values_default_to_not_available() {
    let empty = request_data(r#"{"metadata": {"user_api_key_user_id": "   "}}"#);
    assert_eq!(
        metadata::header_value(&empty, "user_api_key_user_id").unwrap(),
        "N/A"
    );
    assert_eq!(
        metadata::header_value(&empty, "user_api_key_alias").unwrap(),
        "N/A"
    );
}

#[test]
fn metadata_non_object_sections_are_refused() {
    let data = request_data(r#"{"metadata": "not-an-object"}"#);
    let error = metadata::selected_policy_id(&data, 4).unwrap_err();
    assert!(matches!(
        error,
        ZscalerAiGuardError::MetadataShape { name: "metadata" }
    ));
}

#[tokio::test]
async fn no_texts_make_no_request_and_return_the_inputs() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let result = guardrail
        .apply_guardrail(
            inputs(&[]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(result, inputs(&[]));
    assert!(mock.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_request_carries_the_documented_headers_and_body() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let mut args = args();
    args.api_base = Some(format!("{}/v1/detection/execute-policy", mock.base));
    args.send_user_api_key_alias = Some(true);
    args.send_user_api_key_user_id = Some(true);
    args.send_user_api_key_team_id = Some(true);
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let guardrail =
        ZscalerAiGuard::new(args, &environment, base_builder().build().unwrap()).unwrap();
    let data = request_data(
        r#"{
            "metadata": {"user_api_key_metadata": {"user_api_key_user_id": "user-1"}},
            "litellm_metadata": {"user_api_key_alias": "alias-1", "user_api_key_team_id": "team-1"}
        }"#,
    );
    guardrail
        .apply_guardrail(
            inputs(&["hello", "world"]),
            &data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/detection/execute-policy");
    assert_eq!(
        seen[0]
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer oracle-key"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("user-api-key-alias")
            .unwrap()
            .to_str()
            .unwrap(),
        "alias-1"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("user-api-key-team-id")
            .unwrap()
            .to_str()
            .unwrap(),
        "team-1"
    );
    assert_eq!(
        seen[0]
            .headers
            .get("user-api-key-user-id")
            .unwrap()
            .to_str()
            .unwrap(),
        "user-1"
    );
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello world", "policyId": 42})
    );
}

#[tokio::test]
async fn the_policy_id_is_only_sent_when_it_is_at_least_one() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let mut args = args();
    args.api_base = Some(mock.base.clone());
    args.policy_id = Some(0);
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let guardrail =
        ZscalerAiGuard::new(args, &environment, base_builder().build().unwrap()).unwrap();
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello"})
    );
}

#[tokio::test]
async fn a_metadata_policy_id_overrides_the_configured_id() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 7}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello", "policyId": 7})
    );
}

#[tokio::test]
async fn a_null_metadata_policy_id_sends_no_policy_id() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": null}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello"})
    );
}

#[tokio::test]
async fn a_response_direction_is_out() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "DETECT"}"#).await;
    let guardrail = guardrail(&mock.base);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Response,
        )
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "OUT", "content": "hello", "policyId": 42})
    );
}

#[tokio::test]
async fn a_block_answer_raises_400_with_the_transaction_and_detectors() {
    let mock = mock(
        200,
        r#"{
            "statusCode": 200,
            "action": "BLOCK",
            "transactionId": "txn-1",
            "detectorResponses": {
                "detector-b": {"action": "BLOCK"},
                "detector-a": {"action": "ALLOW"},
                "detector-c": {"action": "BLOCK"}
            }
        }"#,
    )
    .await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["attack"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Block {
            error: "Content blocked by Zscaler AI Guard: {'transactionId': 'txn-1', 'blockingDetectors': ['detector-b', 'detector-c']}".to_owned()
        }
    );
}

#[tokio::test]
async fn a_missing_transaction_id_prints_as_none() {
    let mock = mock(
        200,
        r#"{"statusCode": 200, "action": "BLOCK", "detectorResponses": {}}"#,
    )
    .await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["attack"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Block {
            error: "Content blocked by Zscaler AI Guard: {'transactionId': None, 'blockingDetectors': []}".to_owned()
        }
    );
}

async fn passing_answer(action: &str) {
    let reply = format!(r#"{{"statusCode": 200, "action": "{action}"}}"#);
    let mock = mock(200, &reply).await;
    let guardrail = guardrail(&mock.base);
    let result = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(result, inputs(&["hello"]));
}

#[tokio::test]
async fn an_allow_answer_passes() {
    passing_answer("ALLOW").await;
}

#[tokio::test]
async fn a_detect_answer_passes() {
    passing_answer("DETECT").await;
}

#[tokio::test]
async fn a_rate_limit_answer_is_a_500() {
    let mock = mock(429, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Rate limit reached. status_code: 429".to_owned()
        }
    );
}

#[tokio::test]
async fn a_server_error_is_a_500() {
    let mock = mock(503, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Service is unavailable (HTTP 503)".to_owned()
        }
    );
}

#[tokio::test]
async fn a_bad_status_code_in_the_answer_is_a_500() {
    let mock = mock(200, r#"{"statusCode": 400, "errorMsg": "bad policy"}"#).await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "statusCode in response: 400, errorMsg: bad policy".to_owned()
        }
    );
}

#[tokio::test]
async fn an_unknown_action_is_a_500() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "REVIEW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Action field in response is REVIEW, expecting 'ALLOW', 'BLOCK' or 'DETECT'"
                .to_owned()
        }
    );
}

#[tokio::test]
async fn any_other_status_keeps_its_status() {
    let mock = mock(300, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::MULTIPLE_CHOICES);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Response status code: 300".to_owned()
        }
    );
}

#[tokio::test]
async fn a_client_error_is_a_500_like_litellms_http_client() {
    let mock = mock(404, "{}").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ZscalerErrorDetail::Service {
            error_type: super::ERROR_TYPE,
            reason: "Response status code: 404".to_owned()
        }
    );
}

#[tokio::test]
async fn an_invalid_json_answer_is_a_500() {
    let mock = mock(200, "not json").await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::ResponseJson { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_transport_error_is_a_500() {
    let guardrail = guardrail("http://127.0.0.1:1");
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ZscalerAiGuardError::Request { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn printed_text_is_a_string_or_the_value_repr() {
    assert_eq!(str_text(&GuardrailValue::Text("abc".to_owned())), "abc");
    assert_eq!(str_text(&GuardrailValue::SignedInteger(7)), "7");
    assert_eq!(str_text(&GuardrailValue::Null), "None");
    assert_eq!(str_text(&GuardrailValue::Boolean(true)), "True");
}

#[test]
fn printed_repr_renders_each_value_like_python() {
    assert_eq!(repr_text(&GuardrailValue::Null), "None");
    assert_eq!(repr_text(&GuardrailValue::Boolean(false)), "False");
    assert_eq!(repr_text(&GuardrailValue::SignedInteger(-3)), "-3");
    assert_eq!(repr_text(&GuardrailValue::UnsignedInteger(3)), "3");
    assert_eq!(repr_text(&GuardrailValue::Decimal(2.0)), "2.0");
    assert_eq!(repr_text(&GuardrailValue::Decimal(1.25)), "1.25");
    assert_eq!(repr_text(&GuardrailValue::Text("a".to_owned())), "'a'");
    assert_eq!(
        repr_text(&GuardrailValue::Array(vec![
            GuardrailValue::SignedInteger(1),
            GuardrailValue::Null,
        ])),
        "[1, None]"
    );
    let mut fields = BTreeMap::new();
    let _previous = fields.insert("k".to_owned(), GuardrailValue::Text("v".to_owned()));
    assert_eq!(repr_text(&GuardrailValue::Object(fields)), "{'k': 'v'}");
}

#[test]
fn printed_repr_quotes_and_escapes_strings() {
    assert_eq!(quote_text("plain"), "'plain'");
    assert_eq!(quote_text("a space"), "'a space'");
    assert_eq!(quote_text("it's"), "\"it's\"");
    assert_eq!(quote_text("a'b\"c"), r#"'a\'b"c'"#);
    assert_eq!(quote_text("a\\b\n\t"), r"'a\\b\n\t'");
    assert_eq!(quote_text("\u{7}"), r"'\x07'");
    assert_eq!(quote_text("\u{2028}"), r"'\u2028'");
    assert_eq!(quote_text("\u{e0001}"), r"'\U000e0001'");
}

#[test]
fn a_null_litellm_metadata_value_falls_through_to_metadata() {
    let data = request_data(
        r#"{"litellm_metadata": {"user_api_key_alias": null}, "metadata": {"user_api_key_alias": "from-metadata"}}"#,
    );
    assert_eq!(
        metadata::header_value(&data, "user_api_key_alias").unwrap(),
        "from-metadata"
    );
}

#[test]
fn a_null_key_alias_falls_through_to_metadata() {
    let data = request_data(
        r#"{"litellm_metadata": {"user_api_key_key_alias": null}, "metadata": {"user_api_key_alias": "from-metadata"}}"#,
    );
    assert_eq!(
        metadata::header_value(&data, "user_api_key_alias").unwrap(),
        "from-metadata"
    );
}

#[test]
fn a_null_metadata_value_is_not_available() {
    let data = request_data(r#"{"metadata": {"user_api_key_alias": null}}"#);
    assert_eq!(
        metadata::header_value(&data, "user_api_key_alias").unwrap(),
        "N/A"
    );
}

#[test]
fn the_key_alias_fallback_only_applies_to_the_alias() {
    let data = request_data(
        r#"{"litellm_metadata": {"user_api_key_key_alias": "key-alias"}, "metadata": {"user_api_key_team_id": "team-1"}}"#,
    );
    assert_eq!(
        metadata::header_value(&data, "user_api_key_team_id").unwrap(),
        "team-1"
    );
}

#[tokio::test]
async fn an_answer_under_the_size_limit_is_read() {
    let content = "x".repeat(3000);
    let reply =
        format!(r#"{{"statusCode": 200, "action": "ALLOW", "transactionId": "{content}"}}"#);
    let mock = mock(200, &reply).await;
    let guardrail = guardrail(&mock.base);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_decimal_200_status_code_is_accepted() {
    let mock = mock(200, r#"{"statusCode": 200.0, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_decimal_status_code_other_than_200_is_a_500() {
    let mock = mock(200, r#"{"statusCode": 400.0, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_decimal_policy_id_from_metadata_is_sent() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 5.0}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello", "policyId": 5.0})
    );
}

#[tokio::test]
async fn a_fractional_policy_id_below_one_is_not_sent() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 0.5}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello"})
    );
}

#[tokio::test]
async fn a_large_unsigned_policy_id_is_sent() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let data = request_data(r#"{"metadata": {"zguard_policy_id": 18446744073709551615}}"#);
    guardrail
        .apply_guardrail(inputs(&["hello"]), &data, GuardrailInputType::Request)
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&seen[0].body).unwrap(),
        serde_json::json!({"direction": "IN", "content": "hello", "policyId": 18446744073709551615_u64})
    );
}

#[tokio::test]
async fn inputs_pass_through_unchanged() {
    let mock = mock(200, r#"{"statusCode": 200, "action": "ALLOW"}"#).await;
    let guardrail = guardrail(&mock.base);
    let mut given = inputs(&["hello"]);
    let _inserted = given.rest.insert(
        "images".to_owned(),
        serde_json::from_str(r#"["aGVsbG8="]"#).unwrap(),
    );
    let result = guardrail
        .apply_guardrail(
            given.clone(),
            &request_data(r#"{"metadata": {}}"#),
            GuardrailInputType::Request,
        )
        .await
        .unwrap();
    assert_eq!(result, given);
}
