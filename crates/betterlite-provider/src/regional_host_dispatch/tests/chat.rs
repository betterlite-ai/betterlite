use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as LogJson;
use rstest::rstest;
use serde_json::json;

use crate::hosted_platform_dispatch::handler::HandlerError;
use crate::regional_host_dispatch::chat::dispatch;
use crate::regional_host_dispatch::context::CallLogging;
use crate::regional_host_dispatch::error::DispatchError;

use super::support::{
    ChatFixture, ChatRecorder, headers, logging, params, redactor, secret, secret_text,
};

fn dispatch_chat(
    tag: ChatConfigTag,
    fixture: &ChatFixture,
    handler: &ChatRecorder,
    log: &mut CallLogging<'_, '_>,
) -> Result<String, DispatchError> {
    dispatch(tag, &fixture.context(), handler, log).expect("the regional table has this arm")
}

// LiteLLM's five-step GigaChat chain: the request key, the global, the section key, then the API key env and the credentials env
#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-global"),
    Some("sk-section"),
    Some("sk-env"),
    Some("sk-cred"),
    Some("sk-request")
)]
#[case::global_key(
    None,
    Some("sk-global"),
    Some("sk-section"),
    Some("sk-env"),
    Some("sk-cred"),
    Some("sk-global")
)]
#[case::section_key(
    None,
    None,
    Some("sk-section"),
    Some("sk-env"),
    Some("sk-cred"),
    Some("sk-section")
)]
#[case::api_key_env(None, None, None, Some("sk-env"), Some("sk-cred"), Some("sk-env"))]
#[case::credentials_env(None, None, None, None, Some("sk-cred"), Some("sk-cred"))]
#[case::no_key(None, None, None, None, None, None)]
fn gigachat_key_chain(
    #[case] request_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] section_key: Option<&str>,
    #[case] api_key_env: Option<&str>,
    #[case] credentials_env: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("gigachat");
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.gigachat_key = section_key.map(secret);
    fixture.settings.environment.gigachat_api_key = api_key_env.map(secret);
    fixture.settings.environment.gigachat_credentials = credentials_env.map(secret);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::GigaChat, &fixture, &handler, &mut log)
        .expect("the gigachat arm answers");
    assert_eq!(
        secret_text(handler.first().api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[test]
fn gigachat_request_headers_win_over_the_global_headers() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("gigachat");
    fixture.headers = Some(headers(&[("x-request", "1")]));
    fixture.settings.headers = Some(headers(&[("x-global", "2")]));
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::GigaChat, &fixture, &handler, &mut log)
        .expect("the gigachat arm answers");
    assert_eq!(
        handler.first().headers,
        Some(headers(&[("x-request", "1")]))
    );
}

#[test]
fn gigachat_global_headers_fill_in_for_an_empty_request_map() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("gigachat");
    fixture.headers = Some(headers(&[]));
    fixture.settings.headers = Some(headers(&[("x-global", "2")]));
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::GigaChat, &fixture, &handler, &mut log)
        .expect("the gigachat arm answers");
    assert_eq!(handler.first().headers, Some(headers(&[("x-global", "2")])));
}

#[test]
fn gigachat_without_any_headers_passes_an_empty_map() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let fixture = ChatFixture::new();
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::GigaChat, &fixture, &handler, &mut log)
        .expect("the gigachat arm answers");
    assert_eq!(handler.first().headers, Some(headers(&[])));
}

#[test]
fn gigachat_passes_the_base_and_provider_config_as_they_came() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("gigachat");
    fixture.model = String::from("GigaChat");
    fixture.api_base = Some(String::from("https://gigachat.example/v1"));
    fixture.provider_config = Some(ChatConfigTag::GigaChat);
    fixture.optional_params = params(r#"{"temperature":1}"#);
    fixture.litellm_params = params(r#"{"user":"oracle"}"#);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::GigaChat, &fixture, &handler, &mut log)
        .expect("the gigachat arm answers");
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
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("gigachat");
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    let handler = ChatRecorder::failing(503);
    let outcome = dispatch_chat(ChatConfigTag::GigaChat, &fixture, &handler, &mut log);
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

// the MiniMax chain is the one where the environment key comes before the global key
#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-env"),
    Some("sk-global"),
    Some("sk-request")
)]
#[case::env_key(None, Some("sk-env"), Some("sk-global"), Some("sk-env"))]
#[case::global_key(None, None, Some("sk-global"), Some("sk-global"))]
#[case::no_key(None, None, None, None)]
fn minimax_key_chain(
    #[case] request_key: Option<&str>,
    #[case] env_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("minimax");
    fixture.api_key = request_key.map(secret);
    fixture.settings.environment.minimax_api_key = env_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::MinimaxChat, &fixture, &handler, &mut log)
        .expect("the minimax arm answers");
    assert_eq!(
        secret_text(handler.first().api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[rstest]
#[case::request_base(
    Some("http://request.example/v1"),
    Some("http://global.example/v1"),
    Some("http://env.example/v1"),
    "http://request.example/v1"
)]
#[case::global_base(
    None,
    Some("http://global.example/v1"),
    Some("http://env.example/v1"),
    "http://global.example/v1"
)]
#[case::env_base(None, None, Some("http://env.example/v1"), "http://env.example/v1")]
#[case::default_base(None, None, None, "https://api.minimax.io/v1")]
fn minimax_base_chain(
    #[case] request_base: Option<&str>,
    #[case] global_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("minimax");
    fixture.api_base = request_base.map(str::to_owned);
    fixture.settings.api_base = global_base.map(str::to_owned);
    fixture.settings.environment.minimax_api_base = env_base.map(str::to_owned);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::MinimaxChat, &fixture, &handler, &mut log)
        .expect("the minimax arm answers");
    assert_eq!(handler.first().api_base.as_deref(), Some(expected));
}

#[test]
fn minimax_passes_the_request_headers_and_provider_config_as_they_came() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.provider = String::from("minimax");
    fixture.provider_config = Some(ChatConfigTag::MinimaxChat);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::MinimaxChat, &fixture, &handler, &mut log)
        .expect("the minimax arm answers");
    let call = handler.first();
    assert_eq!(call.headers, None);
    assert_eq!(call.provider_config, Some(ChatConfigTag::MinimaxChat));
}

#[test]
fn minimax_logs_the_reply_on_success() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let fixture = ChatFixture::new();
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::MinimaxChat, &fixture, &handler, &mut log)
        .expect("the minimax arm answers");
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&LogJson::Null)
    );
}

#[test]
fn minimax_returns_the_handler_error_without_logging() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let fixture = ChatFixture::new();
    let handler = ChatRecorder::failing(500);
    let outcome = dispatch_chat(ChatConfigTag::MinimaxChat, &fixture, &handler, &mut log);
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 500
        }))
    ));
    assert_eq!(state.model_call_details.get("original_response"), None);
}

#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-section"),
    Some("sk-env"),
    Some("sk-global"),
    Some("sk-request")
)]
#[case::section_key(
    None,
    Some("sk-section"),
    Some("sk-env"),
    Some("sk-global"),
    Some("sk-section")
)]
#[case::env_key(None, None, Some("sk-env"), Some("sk-global"), Some("sk-env"))]
#[case::global_key(None, None, None, Some("sk-global"), Some("sk-global"))]
#[case::no_key(None, None, None, None, None)]
fn nlp_cloud_key_chain(
    #[case] request_key: Option<&str>,
    #[case] section_key: Option<&str>,
    #[case] env_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.api_key = request_key.map(secret);
    fixture.settings.nlp_cloud_key = section_key.map(secret);
    fixture.settings.environment.nlp_cloud_api_key = env_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::NlpCloud, &fixture, &handler, &mut log)
        .expect("the nlp cloud arm answers");
    assert_eq!(
        secret_text(handler.first().api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[rstest]
#[case::request_base(
    Some("http://request.example/v1"),
    Some("http://global.example/v1"),
    Some("http://env.example/v1"),
    "http://request.example/v1"
)]
#[case::global_base(
    None,
    Some("http://global.example/v1"),
    Some("http://env.example/v1"),
    "http://global.example/v1"
)]
#[case::env_base(None, None, Some("http://env.example/v1"), "http://env.example/v1")]
#[case::default_base(None, None, None, "https://api.nlpcloud.io/v1/gpu/")]
fn nlp_cloud_base_chain(
    #[case] request_base: Option<&str>,
    #[case] global_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: &str,
) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.api_base = request_base.map(str::to_owned);
    fixture.settings.api_base = global_base.map(str::to_owned);
    fixture.settings.environment.nlp_cloud_api_base = env_base.map(str::to_owned);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::NlpCloud, &fixture, &handler, &mut log)
        .expect("the nlp cloud arm answers");
    assert_eq!(handler.first().api_base.as_deref(), Some(expected));
}

#[test]
fn nlp_cloud_drops_the_request_headers_and_the_provider_config() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.provider_config = Some(ChatConfigTag::NlpCloud);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::NlpCloud, &fixture, &handler, &mut log)
        .expect("the nlp cloud arm answers");
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
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.optional_params = params(options);
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::NlpCloud, &fixture, &handler, &mut log)
        .expect("the nlp cloud arm answers");
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
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.optional_params = params(options);
    fixture.acompletion = acompletion;
    let handler = ChatRecorder::succeeding();
    dispatch_chat(ChatConfigTag::NlpCloud, &fixture, &handler, &mut log)
        .expect("the nlp cloud arm answers");
    assert_eq!(
        state.model_call_details.get("original_response").is_some(),
        logged
    );
}

#[test]
fn nlp_cloud_returns_a_streaming_handler_error_without_logging() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let mut fixture = ChatFixture::new();
    fixture.optional_params = params(r#"{"stream":true}"#);
    let handler = ChatRecorder::failing(429);
    let outcome = dispatch_chat(ChatConfigTag::NlpCloud, &fixture, &handler, &mut log);
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 429
        }))
    ));
    assert_eq!(state.model_call_details.get("original_response"), None);
}

#[test]
fn providers_outside_the_regional_table_have_no_arm() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let fixture = ChatFixture::new();
    let handler = ChatRecorder::succeeding();
    let context = fixture.context();
    assert!(dispatch(ChatConfigTag::DashScopeChat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::VolcEngine, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::Ai21Chat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::TencentChat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::ModelScopeChat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::OpenAiGpt, &context, &handler, &mut log).is_none());
}
