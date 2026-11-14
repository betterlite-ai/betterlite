#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::compatible_embedding_settings::{Globals, Settings};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::compatible_embedding::Mode;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::{CometapiEmbedding, SharedEmbedding, shared_embedding};
use crate::prepared::PreparedRequest;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"embedding","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const REPLY: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.25]}],"model":"upstream","usage":{"prompt_tokens":1,"total_tokens":1}}"#;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

async fn serve(listener: &TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        let text = String::from_utf8(request.clone()).unwrap();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let length = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|line| line.strip_prefix("content-length: ")?.parse::<usize>().ok())
                .unwrap_or(0);
            if body.len() >= length {
                break;
            }
        }
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{REPLY}",
        REPLY.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    String::from_utf8(request).unwrap()
}

const ENVIRONMENT: &[(&str, &str)] = &[
    ("OPENAI_LIKE_API_BASE", "http://env.example/v1"),
    ("OPENAI_LIKE_API_KEY", "sk-env"),
    ("COMETAPI_API_BASE", "https://comet.example/v1"),
    ("COMETAPI_KEY", "sk-cometapi-env"),
];

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .compatible_embedding_settings
}

fn exposed(value: Option<&SecretString>) -> Option<String> {
    value.map(|secret| secret.expose_secret().to_owned())
}

#[rstest]
#[case::environment_takes_over(
    ENVIRONMENT,
    None,
    None,
    None,
    None,
    None,
    Some("http://env.example/v1"),
    Some("sk-env")
)]
#[case::globals_win(
    ENVIRONMENT,
    Some("https://global/v1"),
    Some("sk-global"),
    None,
    None,
    None,
    Some("https://global/v1"),
    Some("sk-global")
)]
#[case::openai_like_global_key(
    ENVIRONMENT,
    None,
    None,
    Some("sk-like"),
    None,
    None,
    Some("http://env.example/v1"),
    Some("sk-like")
)]
#[case::global_api_key_precedes_the_like_key(
    ENVIRONMENT,
    Some("https://global/v1"),
    Some("sk-global"),
    Some("sk-like"),
    None,
    None,
    Some("https://global/v1"),
    Some("sk-global")
)]
#[case::explicit_empty_key_suppresses(
    ENVIRONMENT,
    None,
    Some("sk-global"),
    None,
    None,
    Some(""),
    Some("http://env.example/v1"),
    Some("")
)]
#[case::explicit_base_wins(
    ENVIRONMENT,
    Some("https://global/v1"),
    None,
    None,
    Some("https://explicit/v1"),
    None,
    Some("https://explicit/v1"),
    Some("sk-env")
)]
#[case::nothing_anywhere(&[], None, None, None, None, None, None, None)]
fn shared_identity_follows_the_pinned_precedence(
    #[case] environment: &[(&str, &str)],
    #[case] global_base: Option<&str>,
    #[case] global_api_key: Option<&str>,
    #[case] global_like_key: Option<&str>,
    #[case] explicit_base: Option<&str>,
    #[case] explicit_key: Option<&str>,
    #[case] expected_base: Option<&str>,
    #[case] expected_key: Option<&str>,
) {
    let input = Json::Text(String::from("a"));
    let options = IndexMap::new();
    let globals = Globals {
        api_base: global_base.map(str::to_owned),
        api_key: global_api_key.map(|text| SecretString::from(text.to_owned())),
        openai_like_key: global_like_key.map(|text| SecretString::from(text.to_owned())),
        cometapi_key: None,
    };
    let settings = settings(environment);
    let supplied_key = explicit_key.map(|text| SecretString::from(text.to_owned()));
    let embedding = SharedEmbedding {
        model: "m",
        input: &input,
        api_base: explicit_base,
        api_key: supplied_key.as_ref(),
        optional_params: &options,
        headers: None,
        custom_endpoint: false,
        timeout: Duration::from_secs(30),
        mode: Mode::Async,
        globals: &globals,
        settings: &settings,
    };
    let call = embedding.handler_call();
    assert_eq!(call.api_base.as_deref(), expected_base);
    assert_eq!(exposed(call.api_key.as_ref()).as_deref(), expected_key);
}

#[rstest]
#[case::nonempty_headers_overwrite(Some(Headers::from([(String::from("x-new"), String::from("2"))])), true)]
#[case::absent_headers_leave_it(None, false)]
#[case::empty_headers_leave_it(Some(Headers::new()), false)]
fn caller_headers_only_replace_extra_headers_when_nonempty(
    #[case] headers: Option<Headers>,
    #[case] replaced: bool,
) {
    let input = Json::Text(String::from("a"));
    let existing = Json::Object(IndexMap::from([(
        String::from("x-old"),
        Json::Text(String::from("1")),
    )]));
    let options = IndexMap::from([(String::from("extra_headers"), existing.clone())]);
    let globals = Globals::default();
    let settings = settings(&[]);
    let embedding = SharedEmbedding {
        model: "m",
        input: &input,
        api_base: Some("https://explicit/v1"),
        api_key: None,
        optional_params: &options,
        headers: headers.as_ref(),
        custom_endpoint: false,
        timeout: Duration::from_secs(30),
        mode: Mode::Async,
        globals: &globals,
        settings: &settings,
    };
    let call = embedding.handler_call();
    let extra_headers = call
        .options
        .get("extra_headers")
        .expect("extra_headers stays");
    if replaced {
        assert_eq!(
            extra_headers,
            &Json::Object(IndexMap::from([(
                String::from("x-new"),
                Json::Text(String::from("2")),
            )]))
        );
    } else {
        assert_eq!(extra_headers, &existing);
    }
}

#[test]
fn the_call_keeps_the_original_input_timeout_mode_and_endpoint_flag() {
    let input = Json::List(vec![Json::Text(String::from("a"))]);
    let options = IndexMap::new();
    let globals = Globals::default();
    let settings = settings(&[]);
    let explicit_key = SecretString::from(String::from("sk-explicit"));
    let embedding = SharedEmbedding {
        model: "m",
        input: &input,
        api_base: Some("https://explicit/v1"),
        api_key: Some(&explicit_key),
        optional_params: &options,
        headers: None,
        custom_endpoint: true,
        timeout: Duration::from_secs(7),
        mode: Mode::Sync,
        globals: &globals,
        settings: &settings,
    };
    let call = embedding.handler_call();
    assert_eq!(call.model, "m");
    assert_eq!(call.input, input);
    assert_eq!(call.timeout, Duration::from_secs(7));
    assert_eq!(call.mode, Mode::Sync);
    assert!(call.custom_endpoint);
    assert!(call.headers.is_none());
}

#[test]
fn cometapi_prepared_request_uses_the_configuration_and_no_caller_headers() {
    let input = PromptJson::List(vec![PromptJson::Text(String::from("a"))]);
    let options: Object = IndexMap::from([
        (
            String::from("dimensions"),
            PromptJson::Number(serde_json::Number::from(2_u32)),
        ),
        (String::from("user"), PromptJson::Text(String::new())),
    ]);
    let globals = Globals::default();
    let settings = settings(ENVIRONMENT);
    let embedding = CometapiEmbedding {
        model: "m",
        input: &input,
        optional_params: &options,
        api_base: None,
        api_key: None,
        timeout: Duration::from_secs(5),
        globals: &globals,
        settings: &settings,
    };
    let PreparedRequest::Embedding(chat) = embedding.prepared().unwrap() else {
        panic!("the CometAPI embedding is a prepared embedding request");
    };
    assert_eq!(
        chat.http.url.as_str(),
        "https://comet.example/v1/embeddings"
    );
    assert_eq!(chat.http.deadline, Duration::from_secs(5));
    assert_eq!(
        chat.http.headers.get("Authorization").unwrap(),
        "Bearer sk-cometapi-env"
    );
    assert_eq!(
        chat.http.headers.get("Content-Type").unwrap(),
        "application/json"
    );
    let body: PromptJson = serde_json::from_slice(&chat.http.body).unwrap();
    assert_eq!(
        body,
        PromptJson::Object(IndexMap::from([
            (
                String::from("input"),
                PromptJson::List(vec![PromptJson::Text(String::from("a"))])
            ),
            (String::from("model"), PromptJson::Text(String::from("m"))),
            (
                String::from("dimensions"),
                PromptJson::Number(serde_json::Number::from(2_u32))
            ),
            (String::from("user"), PromptJson::Text(String::new())),
        ]))
    );
}

#[rstest]
#[case::environment(
    ENVIRONMENT,
    None,
    None,
    None,
    None,
    "https://comet.example/v1/embeddings",
    "Bearer sk-cometapi-env"
)]
#[case::default_base_and_no_key(&[], None, None, None, None, "https://api.cometapi.com/v1/embeddings", "Bearer None")]
#[case::global_cometapi_key(
    ENVIRONMENT,
    None,
    None,
    Some("sk-cometapi-global"),
    None,
    "https://comet.example/v1/embeddings",
    "Bearer sk-cometapi-global"
)]
#[case::general_global_key_loses_to_the_alias(
    ENVIRONMENT,
    None,
    Some("sk-api"),
    None,
    None,
    "https://comet.example/v1/embeddings",
    "Bearer sk-cometapi-env"
)]
#[case::global_base_wins(
    ENVIRONMENT,
    Some("https://global/v1"),
    None,
    None,
    None,
    "https://global/v1/embeddings",
    "Bearer sk-cometapi-env"
)]
#[case::explicit_wins_with_trailing_slash(ENVIRONMENT, Some("https://global/v1"), Some("sk-api"), None, Some(("https://explicit/v1/", "sk-explicit")), "https://explicit/v1/embeddings", "Bearer sk-explicit")]
fn cometapi_identity_follows_the_pinned_precedence(
    #[case] environment: &[(&str, &str)],
    #[case] global_base: Option<&str>,
    #[case] global_api_key: Option<&str>,
    #[case] global_cometapi_key: Option<&str>,
    #[case] explicit: Option<(&str, &str)>,
    #[case] expected_base: &str,
    #[case] expected_authorization: &str,
) {
    let input = PromptJson::Text(String::from("a"));
    let options = Object::new();
    let globals = Globals {
        api_base: global_base.map(str::to_owned),
        api_key: global_api_key.map(|text| SecretString::from(text.to_owned())),
        openai_like_key: None,
        cometapi_key: global_cometapi_key.map(|text| SecretString::from(text.to_owned())),
    };
    let settings = settings(environment);
    let supplied_key = explicit.map(|(_, key)| SecretString::from(key.to_owned()));
    let embedding = CometapiEmbedding {
        model: "m",
        input: &input,
        optional_params: &options,
        api_base: explicit.map(|(base, _)| base),
        api_key: supplied_key.as_ref(),
        timeout: Duration::from_secs(5),
        globals: &globals,
        settings: &settings,
    };
    let PreparedRequest::Embedding(chat) = embedding.prepared().unwrap() else {
        panic!("the CometAPI embedding is a prepared embedding request");
    };
    assert_eq!(chat.http.url.as_str(), expected_base);
    assert_eq!(
        chat.http.headers.get("Authorization").unwrap(),
        expected_authorization
    );
}

#[tokio::test]
async fn cometapi_execute_sends_the_prepared_request_and_keeps_the_callers_model() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let input = PromptJson::Text(String::from("a"));
    let options = Object::new();
    let globals = Globals::default();
    let settings = settings(ENVIRONMENT);
    let key = SecretString::from(String::from("sk-explicit"));
    let embedding = CometapiEmbedding {
        model: "m",
        input: &input,
        optional_params: &options,
        api_base: Some(&base),
        api_key: Some(&key),
        timeout: Duration::from_secs(5),
        globals: &globals,
        settings: &settings,
    };
    let client = client();
    let (request, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener), embedding.execute(&client))
    })
    .await
    .unwrap();
    let response = answer.unwrap();
    assert_eq!(response.model, "m");
    assert_eq!(response.data.len(), 1);
    assert!(request.starts_with("POST /v1/embeddings HTTP/1.1"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-explicit")
    );
    assert!(request.contains(r#""model":"m""#));
}

#[tokio::test]
async fn shared_embedding_sends_the_handler_call_and_returns_the_decoded_reply() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let input = Json::List(vec![Json::Text(String::from("a"))]);
    let options = IndexMap::new();
    let globals = Globals::default();
    let settings = settings(&[]);
    let key = SecretString::from(String::from("sk-explicit"));
    let call = SharedEmbedding {
        model: "m",
        input: &input,
        api_base: Some(&base),
        api_key: Some(&key),
        optional_params: &options,
        headers: None,
        custom_endpoint: false,
        timeout: Duration::from_secs(5),
        mode: Mode::Async,
        globals: &globals,
        settings: &settings,
    }
    .handler_call();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "trace".to_owned(),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    let client = client();
    let (request, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(
            serve(&listener),
            shared_embedding(call, &client, &mut state, &env)
        )
    })
    .await
    .unwrap();
    let response = answer.unwrap();
    assert_eq!(response.model, "upstream");
    assert_eq!(response.data.len(), 1);
    assert!(request.starts_with("POST /v1/embeddings HTTP/1.1"));
    assert!(request.contains(r#""input":["a"]"#));
}
