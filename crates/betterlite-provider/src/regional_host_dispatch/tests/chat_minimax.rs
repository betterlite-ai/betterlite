use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as LogJson;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::hosted_platform_dispatch::handler::HandlerError;
use crate::regional_host_dispatch::error::DispatchError;

use super::support::{ChatFixture, ChatRecorder, chat_dispatch, headers, secret};

struct Sources {
    request_key: Option<&'static str>,
    env_key: Option<&'static str>,
    global_key: Option<&'static str>,
    request_base: Option<&'static str>,
    global_base: Option<&'static str>,
    env_base: Option<&'static str>,
}

// the MiniMax chain: the environment key comes before the global key, and the public base is the last fallback
#[rstest]
#[case::request_sources(
    Sources {
        request_key: Some("sk-request"),
        env_key: Some("sk-env"),
        global_key: Some("sk-global"),
        request_base: Some("http://request.example/v1"),
        global_base: Some("http://global.example/v1"),
        env_base: Some("http://env.example/v1"),
    },
    Some("sk-request"),
    "http://request.example/v1"
)]
#[case::env_key_beats_global(
    Sources {
        request_key: None,
        env_key: Some("sk-env"),
        global_key: Some("sk-global"),
        request_base: None,
        global_base: Some("http://global.example/v1"),
        env_base: Some("http://env.example/v1"),
    },
    Some("sk-env"),
    "http://global.example/v1"
)]
#[case::env_sources(
    Sources {
        request_key: None,
        env_key: Some("sk-env"),
        global_key: None,
        request_base: None,
        global_base: None,
        env_base: Some("http://env.example/v1"),
    },
    Some("sk-env"),
    "http://env.example/v1"
)]
#[case::global_sources(
    Sources {
        request_key: None,
        env_key: None,
        global_key: Some("sk-global"),
        request_base: None,
        global_base: Some("http://global.example/v1"),
        env_base: None,
    },
    Some("sk-global"),
    "http://global.example/v1"
)]
#[case::no_key(
    Sources {
        request_key: None,
        env_key: None,
        global_key: None,
        request_base: None,
        global_base: None,
        env_base: None,
    },
    None,
    "https://api.minimax.io/v1"
)]
fn minimax_chain(
    #[case] sources: Sources,
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("minimax");
    fixture.api_key = sources.request_key.map(secret);
    fixture.settings.environment.minimax_api_key = sources.env_key.map(secret);
    fixture.settings.api_key = sources.global_key.map(secret);
    fixture.api_base = sources.request_base.map(str::to_owned);
    fixture.settings.api_base = sources.global_base.map(str::to_owned);
    fixture.settings.environment.minimax_api_base = sources.env_base.map(str::to_owned);
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::MinimaxChat, &fixture, &handler);
    outcome.expect("the minimax arm answers");
    let call = handler.first();
    assert_eq!(
        call.api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        expected_key.map(str::to_owned)
    );
    assert_eq!(call.api_base.as_deref(), Some(expected_base));
}

// an empty string is falsy in LiteLLM's `api_key or litellm.api_key` chain, so each step skips it
#[test]
fn minimax_skips_an_empty_request_key_and_base() {
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("minimax");
    fixture.api_key = Some(secret(""));
    fixture.settings.api_key = Some(secret("sk-global"));
    fixture.api_base = Some(String::new());
    fixture.settings.api_base = Some(String::from("http://global.example/v1"));
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::MinimaxChat, &fixture, &handler);
    outcome.expect("the minimax arm answers");
    let call = handler.first();
    assert_eq!(
        call.api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("sk-global"))
    );
    assert_eq!(call.api_base.as_deref(), Some("http://global.example/v1"));
}

#[test]
fn minimax_passes_the_request_headers_and_provider_config_as_they_came() {
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("minimax");
    fixture.provider_config = Some(ChatConfigTag::MinimaxChat);
    fixture.headers = Some(headers(&[("x-request", "1")]));
    fixture.settings.headers = Some(headers(&[("x-global", "2")]));
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::MinimaxChat, &fixture, &handler);
    outcome.expect("the minimax arm answers");
    let call = handler.first();
    assert_eq!(call.headers, Some(headers(&[("x-request", "1")])));
    assert_eq!(call.provider_config, Some(ChatConfigTag::MinimaxChat));
}

#[test]
fn minimax_logs_the_reply_on_success() {
    let fixture = ChatFixture::new();
    let handler = ChatRecorder::succeeding();
    let (outcome, state) = chat_dispatch(ChatConfigTag::MinimaxChat, &fixture, &handler);
    outcome.expect("the minimax arm answers");
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&LogJson::Null)
    );
    let input = serde_json::from_value::<LogJson>(
        serde_json::to_value(&fixture.messages).expect("the fixture messages serialize"),
    )
    .expect("the fixture messages are JSON");
    assert_eq!(state.model_call_details.get("input"), Some(&input));
}

#[test]
fn minimax_returns_the_handler_error_without_logging() {
    let fixture = ChatFixture::new();
    let handler = ChatRecorder::failing(500);
    let (outcome, state) = chat_dispatch(ChatConfigTag::MinimaxChat, &fixture, &handler);
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 500
        }))
    ));
    assert_eq!(state.model_call_details.get("original_response"), None);
}
