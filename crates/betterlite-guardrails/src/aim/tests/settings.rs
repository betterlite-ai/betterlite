use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use futures_util::stream;
use reqwest::StatusCode;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::{Fake, KEY, State, data, fixture, guardrail_with};
use crate::aim::error::AimError;
use crate::aim::input;
use crate::aim::settings::{
    AimGuardrail, AimHook, AimParams, DEFAULT_TIMEOUT, SUPPORTED_EVENT_HOOKS,
};
use crate::aim::stream::{
    AimSocketConnector, CONNECT_TIMEOUT, MAX_STREAM_FRAME_BYTES, inspect_output_stream,
};
use crate::aim::transport::AimPoster;

#[rstest]
#[case::pre_call(AimHook::PreCall, "pre_call")]
#[case::moderation(AimHook::Moderation, "moderation")]
#[case::output(AimHook::Output, "output")]
fn aim_guardrail_names_the_hook_in_its_header(#[case] hook: AimHook, #[case] expected: &str) {
    let guardrail = fixture(StatusCode::OK, "{}", &[]).guardrail;
    let headers = guardrail.build_headers(hook, None, None, None).unwrap();
    assert_eq!(headers.get("x-aim-litellm-hook").unwrap(), expected);
}

#[rstest]
#[case::absent(None, None, None)]
#[case::empty(Some(""), Some(""), Some(""))]
fn aim_guardrail_omits_empty_and_absent_optional_headers(
    #[case] alias: Option<&str>,
    #[case] email: Option<&str>,
    #[case] call_id: Option<&str>,
) {
    let guardrail = fixture(StatusCode::OK, "{}", &[]).guardrail;
    let headers = guardrail
        .build_headers(AimHook::PreCall, alias, email, call_id)
        .unwrap();
    assert_eq!(headers.len(), 3);
    assert!(!headers.contains_key("x-aim-call-id"));
    assert!(!headers.contains_key("x-aim-user-email"));
    assert!(!headers.contains_key("x-aim-gateway-key-alias"));
}

#[test]
fn aim_guardrail_sends_the_optional_headers_when_present() {
    let guardrail = fixture(StatusCode::OK, "{}", &[]).guardrail;
    let headers = guardrail
        .build_headers(AimHook::PreCall, Some("a"), Some("e"), Some("c"))
        .unwrap();
    assert_eq!(headers.get("x-aim-gateway-key-alias").unwrap(), "a");
    assert_eq!(headers.get("x-aim-user-email").unwrap(), "e");
    assert_eq!(headers.get("x-aim-call-id").unwrap(), "c");
}

#[test]
fn aim_guardrail_rejects_an_invalid_header_value() {
    let guardrail = fixture(StatusCode::OK, "{}", &[]).guardrail;
    let invalid = guardrail.build_headers(AimHook::PreCall, Some("bad\nalias"), None, None);
    assert!(matches!(invalid, Err(AimError::InvalidHeader(_))));
}

#[test]
fn aim_guardrail_ignores_empty_and_malformed_call_fields() {
    let empty =
        data(r#"{"litellm_call_id": "", "metadata": {"headers": {"x-aim-user-email": ""}}}"#);
    assert_eq!(input::text_field(&empty, "litellm_call_id"), None);
    assert_eq!(input::user_email(&empty), None);
    let absent = data(r#"{"metadata": {"headers": "none"}}"#);
    assert_eq!(input::user_email(&absent), None);
}

#[rstest]
#[case::default_base(&[("AIM_API_KEY", "k")], None, "https://api.aim.security", "wss://api.aim.security")]
#[case::env_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "http://env.aim")], None, "http://env.aim", "ws://env.aim")]
#[case::empty_env_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "")], None, "https://api.aim.security", "wss://api.aim.security")]
#[case::param_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "http://env.aim")], Some("https://param.aim"), "https://param.aim", "wss://param.aim")]
#[case::empty_param_base(&[("AIM_API_KEY", "k"), ("AIM_API_BASE", "http://env.aim")], Some(""), "http://env.aim", "ws://env.aim")]
fn aim_guardrail_resolves_the_base_and_the_websocket_base(
    #[case] variables: &[(&str, &str)],
    #[case] param: Option<&str>,
    #[case] base: &str,
    #[case] websocket_base: &str,
) {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let params = AimParams {
        guardrail_name: None,
        api_key: None,
        api_base: param.map(str::to_owned),
        inspect_embeddings: None,
        litellm_version: None,
    };
    let state = Arc::new(State::default());
    let guardrail = guardrail_with(params, &environment, &state, (StatusCode::OK, "{}")).unwrap();
    assert_eq!(guardrail.api_base, base);
    assert_eq!(guardrail.ws_api_base, websocket_base);
    assert_eq!(guardrail.litellm_version, "1.105.1");
    assert!(!guardrail.inspect_embeddings);
}

#[rstest]
#[case::configured(Some("param-key"), "param-key")]
#[case::empty(Some(""), "env-key")]
#[case::absent(None, "env-key")]
fn aim_guardrail_prefers_the_configured_api_key_over_the_environment(
    #[case] param: Option<&str>,
    #[case] expected: &str,
) {
    let environment = BTreeMap::from([("AIM_API_KEY".to_owned(), "env-key".to_owned())]);
    let state = Arc::new(State::default());
    let params = AimParams {
        guardrail_name: None,
        api_key: param.map(|key| SecretString::from(key.to_owned())),
        api_base: None,
        inspect_embeddings: None,
        litellm_version: None,
    };
    let guardrail = guardrail_with(params, &environment, &state, (StatusCode::OK, "{}")).unwrap();
    assert_eq!(guardrail.api_key.expose_secret(), expected);
}

#[test]
fn aim_guardrail_supports_the_conversational_hooks() {
    let hooks = serde_json::to_value(SUPPORTED_EVENT_HOOKS).unwrap();
    assert_eq!(hooks, json!(["pre_call", "during_call", "post_call"]));
}

#[test]
fn aim_guardrail_missing_api_key_is_an_error() {
    let state = Arc::new(State::default());
    let result = guardrail_with(
        AimParams::default(),
        &BTreeMap::new(),
        &state,
        (StatusCode::OK, "{}"),
    );
    assert!(matches!(result, Err(AimError::MissingSecrets)));
}

#[test]
fn aim_guardrail_stream_frame_limit_is_one_mebibyte() {
    assert_eq!(MAX_STREAM_FRAME_BYTES, 0x0010_0000);
}

#[tokio::test]
async fn aim_guardrail_stream_defaults_the_frame_wait_to_the_guardrail_timeout() {
    let state = Arc::new(State::default());
    let environment = BTreeMap::from([("AIM_API_KEY".to_owned(), KEY.to_owned())]);
    let fake = Arc::new(Fake {
        state: Arc::clone(&state),
        status: StatusCode::OK,
        body: Vec::new(),
    });
    let shared: Arc<Fake> = Arc::clone(&fake);
    let poster: Arc<dyn AimPoster> = shared;
    let connector: Arc<dyn AimSocketConnector> = fake;
    let guardrail =
        AimGuardrail::new(AimParams::default(), &environment, poster, connector, None).unwrap();
    let request = data(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    let _stream = inspect_output_stream(&guardrail, stream::empty(), &request, None)
        .await
        .unwrap();
    let timings = state.timings.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(*timings, vec![(CONNECT_TIMEOUT, DEFAULT_TIMEOUT)]);
}
