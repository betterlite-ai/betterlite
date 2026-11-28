use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use rstest::rstest;

use crate::hosted_platform_dispatch::handler::HandlerError;
use crate::regional_host_dispatch::error::DispatchError;

use super::support::{
    ChatFixture, ChatRecorder, chat_dispatch, headers, params, secret, secret_text,
};

struct Sources {
    request_key: Option<&'static str>,
    section_key: Option<&'static str>,
    env_key: Option<&'static str>,
    global_key: Option<&'static str>,
    request_base: Option<&'static str>,
    global_base: Option<&'static str>,
    env_base: Option<&'static str>,
}

// the NLP Cloud chain: the section key comes before the environment and the global key, and the GPU base is the last fallback
#[rstest]
#[case::request_sources(
    Sources {
        request_key: Some("sk-request"),
        section_key: Some("sk-section"),
        env_key: Some("sk-env"),
        global_key: Some("sk-global"),
        request_base: Some("http://request.example/v1"),
        global_base: Some("http://global.example/v1"),
        env_base: Some("http://env.example/v1"),
    },
    Some("sk-request"),
    "http://request.example/v1"
)]
#[case::section_key_beats_env_and_global(
    Sources {
        request_key: None,
        section_key: Some("sk-section"),
        env_key: Some("sk-env"),
        global_key: Some("sk-global"),
        request_base: None,
        global_base: Some("http://global.example/v1"),
        env_base: Some("http://env.example/v1"),
    },
    Some("sk-section"),
    "http://global.example/v1"
)]
#[case::env_key_beats_global(
    Sources {
        request_key: None,
        section_key: None,
        env_key: Some("sk-env"),
        global_key: Some("sk-global"),
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
        section_key: None,
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
        section_key: None,
        env_key: None,
        global_key: None,
        request_base: None,
        global_base: None,
        env_base: None,
    },
    None,
    "https://api.nlpcloud.io/v1/gpu/"
)]
fn nlp_cloud_chain(
    #[case] sources: Sources,
    #[case] expected_key: Option<&str>,
    #[case] expected_base: &str,
) {
    let mut fixture = ChatFixture::new();
    fixture.api_key = sources.request_key.map(secret);
    fixture.settings.nlp_cloud_key = sources.section_key.map(secret);
    fixture.settings.environment.nlp_cloud_api_key = sources.env_key.map(secret);
    fixture.settings.api_key = sources.global_key.map(secret);
    fixture.api_base = sources.request_base.map(str::to_owned);
    fixture.settings.api_base = sources.global_base.map(str::to_owned);
    fixture.settings.environment.nlp_cloud_api_base = sources.env_base.map(str::to_owned);
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::NlpCloud, &fixture, &handler);
    outcome.expect("the nlp cloud arm answers");
    let call = handler.first();
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected_key.map(str::to_owned)
    );
    assert_eq!(call.api_base.as_deref(), Some(expected_base));
}

#[test]
fn nlp_cloud_drops_the_request_headers_and_the_provider_config() {
    let mut fixture = ChatFixture::new();
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.provider_config = Some(ChatConfigTag::NlpCloud);
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::NlpCloud, &fixture, &handler);
    outcome.expect("the nlp cloud arm answers");
    let call = handler.first();
    assert_eq!(call.headers, None);
    assert_eq!(call.provider_config, None);
}

// LiteLLM tests `optional_params["stream"] is True`, so only a JSON boolean asks for the cleaner
#[rstest]
#[case::stream_true(r#"{"stream":true}"#, Some(true))]
#[case::stream_false(r#"{"stream":false}"#, Some(false))]
#[case::absent("{}", Some(false))]
#[case::string(r#"{"stream":"true"}"#, Some(false))]
fn nlp_cloud_sets_the_stream_flag_from_optional_params(
    #[case] options: &str,
    #[case] expected: Option<bool>,
) {
    let mut fixture = ChatFixture::new();
    fixture.optional_params = params(options);
    let handler = ChatRecorder::succeeding();
    let (outcome, _state) = chat_dispatch(ChatConfigTag::NlpCloud, &fixture, &handler);
    outcome.expect("the nlp cloud arm answers");
    assert_eq!(handler.first().stream, expected);
}

#[rstest]
#[case::streaming(r#"{"stream":true}"#, false, true)]
#[case::async_call("{}", true, true)]
#[case::sync_call("{}", false, false)]
fn nlp_cloud_logs_only_for_a_streaming_or_async_call(
    #[case] options: &str,
    #[case] acompletion: bool,
    #[case] logged: bool,
) {
    let mut fixture = ChatFixture::new();
    fixture.optional_params = params(options);
    fixture.acompletion = acompletion;
    let handler = ChatRecorder::succeeding();
    let (outcome, state) = chat_dispatch(ChatConfigTag::NlpCloud, &fixture, &handler);
    outcome.expect("the nlp cloud arm answers");
    assert_eq!(
        state.model_call_details.get("original_response").is_some(),
        logged
    );
}

#[test]
fn nlp_cloud_returns_a_streaming_handler_error_without_logging() {
    let mut fixture = ChatFixture::new();
    fixture.optional_params = params(r#"{"stream":true}"#);
    let handler = ChatRecorder::failing(429);
    let (outcome, state) = chat_dispatch(ChatConfigTag::NlpCloud, &fixture, &handler);
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 429
        }))
    ));
    assert_eq!(state.model_call_details.get("original_response"), None);
}
