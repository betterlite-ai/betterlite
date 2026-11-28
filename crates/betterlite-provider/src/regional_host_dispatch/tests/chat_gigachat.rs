use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as LogJson;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::json;

use crate::hosted_platform_dispatch::handler::HandlerError;
use crate::regional_host_dispatch::error::DispatchError;

use super::support::{ChatFixture, ChatRecorder, chat_dispatch, headers, params, secret};

// LiteLLM's five-step GigaChat chain: the request key, the global, the section key, then the API key env and the credentials env
#[rstest]
#[case::request_key(
    [Some("sk-request"), Some("sk-global"), Some("sk-section"), Some("sk-env"), Some("sk-cred")],
    Some("sk-request")
)]
#[case::global_key(
    [None, Some("sk-global"), Some("sk-section"), Some("sk-env"), Some("sk-cred")],
    Some("sk-global")
)]
#[case::section_key(
    [None, None, Some("sk-section"), Some("sk-env"), Some("sk-cred")],
    Some("sk-section")
)]
#[case::api_key_env([None, None, None, Some("sk-env"), Some("sk-cred")], Some("sk-env"))]
#[case::credentials_env([None, None, None, None, Some("sk-cred")], Some("sk-cred"))]
#[case::no_key([None, None, None, None, None], None)]
fn gigachat_key_chain(#[case] sources: [Option<&str>; 5], #[case] expected: Option<&str>) {
    let [
        request_key,
        global_key,
        section_key,
        api_key_env,
        credentials_env,
    ] = sources;
    let mut fixture = ChatFixture::default();
    fixture.provider = String::from("gigachat");
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.gigachat_key = section_key.map(secret);
    fixture.settings.environment.gigachat_api_key = api_key_env.map(secret);
    fixture.settings.environment.gigachat_credentials = credentials_env.map(secret);
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::GigaChat, &fixture, &handler);
    outcome.expect("the gigachat arm answers");
    assert_eq!(
        handler
            .first()
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        expected.map(str::to_owned)
    );
}

#[test]
fn gigachat_request_headers_win_over_the_global_headers() {
    let mut fixture = ChatFixture::default();
    fixture.provider = String::from("gigachat");
    fixture.headers = Some(headers(&[("x-request", "1")]));
    fixture.settings.headers = Some(headers(&[("x-global", "2")]));
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::GigaChat, &fixture, &handler);
    outcome.expect("the gigachat arm answers");
    assert_eq!(
        handler.first().headers,
        Some(headers(&[("x-request", "1")]))
    );
}

#[test]
fn gigachat_global_headers_fill_in_for_an_empty_request_map() {
    let mut fixture = ChatFixture::default();
    fixture.provider = String::from("gigachat");
    fixture.headers = Some(headers(&[]));
    fixture.settings.headers = Some(headers(&[("x-global", "2")]));
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::GigaChat, &fixture, &handler);
    outcome.expect("the gigachat arm answers");
    assert_eq!(handler.first().headers, Some(headers(&[("x-global", "2")])));
}

#[test]
fn gigachat_without_any_headers_passes_an_empty_map() {
    let fixture = ChatFixture::default();
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::GigaChat, &fixture, &handler);
    outcome.expect("the gigachat arm answers");
    assert_eq!(handler.first().headers, Some(headers(&[])));
}

#[test]
fn gigachat_passes_the_base_and_provider_config_as_they_came() {
    let mut fixture = ChatFixture::default();
    fixture.provider = String::from("gigachat");
    fixture.model = String::from("GigaChat");
    fixture.api_base = Some(String::from("https://gigachat.example/v1"));
    fixture.provider_config = Some(ChatConfigTag::GigaChat);
    fixture.optional_params = params(r#"{"temperature":1}"#);
    fixture.litellm_params = params(r#"{"user":"oracle"}"#);
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::GigaChat, &fixture, &handler);
    outcome.expect("the gigachat arm answers");
    let call = handler.first();
    assert_eq!(call.api_base, fixture.api_base);
    assert_eq!(call.provider_config, Some(ChatConfigTag::GigaChat));
    assert_eq!(call.model, "GigaChat");
    assert_eq!(call.custom_llm_provider, "gigachat");
    assert_eq!(call.optional_params, fixture.optional_params);
    assert_eq!(call.litellm_params, fixture.litellm_params);
    assert_eq!(call.messages, fixture.messages);
}

#[test]
fn gigachat_logs_the_handler_error_with_the_resolved_headers() {
    let mut fixture = ChatFixture::default();
    fixture.provider = String::from("gigachat");
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    let handler = ChatRecorder::failing(503);
    let (outcome, state) = chat_dispatch(ChatConfigTag::GigaChat, &fixture, &handler);
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 503
        }))
    ));
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&LogJson::Text(String::from(
            "the provider answered with status 503"
        )))
    );
    let args = serde_json::to_value(
        state
            .model_call_details
            .get("additional_args")
            .expect("the failure logged its additional args"),
    )
    .expect("the additional args serialize");
    assert_eq!(
        args.get("headers").and_then(|given| given.get("x-scope")),
        Some(&json!("tenant"))
    );
}
