#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
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
use betterlite_openai::chat::{ChatMessage, MessageContent};
use betterlite_openai::compatible_line_stream::Mode;
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use chrono::DateTime;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::error::CompatibleChatError;
use super::execution::{CompatibleChatExecution, MessageTransformer, TimeoutFacts};
use super::plan::ChatPlan;
use super::send::{ChatAnswer, CompatibleChatOutcome, finish_chat, send_chat};
use super::stream::{Bytes, CompatibleChatStream, DecodedChunks, StreamingDecoder};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_TIMEOUT: TimeoutFacts = TimeoutFacts {
    total: Duration::from_secs(600),
    connect: Duration::from_secs(5),
};
const UNIFORM_30: TimeoutFacts = TimeoutFacts {
    total: Duration::from_secs(30),
    connect: Duration::from_secs(30),
};

const REPLY: &str = r#"{"id":"chat-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const FAKE_REPLY: &str = r#"{"id":"chat-1","object":"chat.completion","created":1,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"fake"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#;
const LINES: &str =
    "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"index\":0}]}\n\ndata: [DONE]\n\n";

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn logging(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
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
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap(),
    };
    (state, env)
}

fn messages() -> Vec<ChatMessage> {
    serde_json::from_value(serde_json::json!([{"role": "user", "content": "hi"}])).unwrap()
}

fn user_message(content: &str) -> Json {
    serde_json::from_value(serde_json::json!([{"role": "user", "content": content}])).unwrap()
}

fn number(value: f64) -> Json {
    Json::Number(serde_json::Number::from_f64(value).unwrap())
}

fn empty_body() -> Bytes {
    futures_util::stream::empty::<Result<Vec<u8>, HttpError>>().boxed()
}

fn empty_chunk() -> GenericChunk {
    GenericChunk {
        text: String::new(),
        tool_use: None,
        is_finished: false,
        finish_reason: String::new(),
        usage: None,
        index: None,
        provider_specific_fields: None,
    }
}

struct Call {
    model: String,
    messages: Vec<ChatMessage>,
    provider: String,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    headers: Option<Headers>,
    options: IndexMap<String, Json>,
    custom_endpoint: Option<bool>,
    timeout: Option<Duration>,
    mode: Mode,
    fake_stream: bool,
}

impl Call {
    fn new(mode: Mode) -> Self {
        Self {
            model: String::from("gpt-4o"),
            messages: messages(),
            provider: String::from("openai_like"),
            api_base: Some(String::from("https://example.test/v1")),
            api_key: Some(SecretString::from(String::from("sk-test"))),
            headers: None,
            options: IndexMap::new(),
            custom_endpoint: None,
            timeout: None,
            mode,
            fake_stream: false,
        }
    }

    fn execution<'a>(
        &'a self,
        transform: Option<&'a dyn MessageTransformer>,
        decoder: Option<&'a dyn StreamingDecoder>,
    ) -> CompatibleChatExecution<'a> {
        CompatibleChatExecution {
            model: &self.model,
            messages: &self.messages,
            custom_llm_provider: &self.provider,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            headers: self.headers.as_ref(),
            optional_params: &self.options,
            custom_endpoint: self.custom_endpoint,
            timeout: self.timeout,
            mode: self.mode,
            fake_stream: self.fake_stream,
            transform_messages: transform,
            streaming_decoder: decoder,
        }
    }
}

fn plan<'a>(call: &'a Call, state: &mut LoggingState, env: &PrePostEnv<'_>) -> ChatPlan<'a> {
    call.execution(None, None).plan(state, env).unwrap()
}

async fn serve(listener: &TcpListener, status: u16, reply: &[u8]) -> String {
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
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reply.len()
    );
    let mut response = head.into_bytes();
    response.extend_from_slice(reply);
    stream.write_all(&response).await.unwrap();
    String::from_utf8(request).unwrap()
}

async fn base(listener: &TcpListener) -> String {
    format!("http://{}/v1", listener.local_addr().unwrap())
}

#[test]
fn plan_consumes_options_and_extra_body_wins() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.options = IndexMap::from([
        ("temperature".to_owned(), number(0.2)),
        (
            "max_retries".to_owned(),
            Json::Number(serde_json::Number::from(3_u32)),
        ),
        (
            "base_model".to_owned(),
            Json::Text(String::from("upstream")),
        ),
        ("json_mode".to_owned(), Json::Bool(true)),
        ("stream".to_owned(), Json::Bool(true)),
        (
            "extra_body".to_owned(),
            Json::Object(IndexMap::from([
                ("temperature".to_owned(), number(0.9)),
                ("top_p".to_owned(), number(0.1)),
            ])),
        ),
    ]);
    let plan = plan(&call, &mut state, &env);
    assert_eq!(plan.url, "https://example.test/v1/chat/completions");
    assert_eq!(plan.body.get("temperature"), Some(&number(0.9)));
    assert_eq!(plan.body.get("top_p"), Some(&number(0.1)));
    assert_eq!(plan.body.get("messages"), Some(&user_message("hi")));
    assert!(!plan.body.contains_key("max_retries"));
    assert!(!plan.body.contains_key("base_model"));
    assert!(!plan.body.contains_key("extra_body"));
    assert!(!plan.body.contains_key("json_mode"));
    assert_eq!(plan.base_model.as_deref(), Some("upstream"));
    assert!(plan.json_mode);
    assert_eq!(plan.body.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("pre_api_call")))
    );
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the pre-call arguments");
    };
    let Some(Json::Object(headers)) = args.get("headers") else {
        panic!("the logged headers");
    };
    assert_eq!(
        headers.get("Content-Type"),
        Some(&Json::Text(String::from("application/json")))
    );
    assert!(headers.contains_key("Authorization"));
}

#[test]
fn a_truthy_custom_endpoint_argument_leaves_the_option_in_the_body() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Sync);
    call.api_base = Some(String::from("https://example.test/custom"));
    call.custom_endpoint = Some(true);
    call.options
        .insert("custom_endpoint".to_owned(), Json::Bool(true));
    let plan = plan(&call, &mut state, &env);
    assert_eq!(plan.url, "https://example.test/custom");
    assert_eq!(plan.body.get("custom_endpoint"), Some(&Json::Bool(true)));
}

#[rstest]
#[case(None, true)]
#[case(Some(false), true)]
#[case(Some(false), false)]
fn a_false_or_absent_custom_endpoint_argument_pops_the_option(
    #[case] argument: Option<bool>,
    #[case] option: bool,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Sync);
    call.custom_endpoint = argument;
    call.options
        .insert("custom_endpoint".to_owned(), Json::Bool(option));
    let plan = plan(&call, &mut state, &env);
    assert!(!plan.body.contains_key("custom_endpoint"));
    if option {
        assert_eq!(plan.url, "https://example.test/v1");
    } else {
        assert_eq!(plan.url, "https://example.test/v1/chat/completions");
    }
}

#[rstest]
#[case("stream", Json::Text(String::from("yes")))]
#[case("base_model", Json::Number(serde_json::Number::from(1_u32)))]
#[case("extra_body", Json::List(Vec::new()))]
fn a_wrong_option_type_names_the_field(#[case] field: &str, #[case] value: Json) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.options.insert(field.to_owned(), value);
    let refused = call.execution(None, None).plan(&mut state, &env);
    let Err(error) = refused else {
        panic!("a wrong option type is an error");
    };
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        error,
        CompatibleChatError::OptionType {
            field: named,
            ..
        } if named == field
    ));
}

#[rstest]
#[case(true, None, None)]
#[case(true, Some(true), Some(true))]
#[case(true, Some(false), Some(false))]
#[case(false, None, Some(true))]
fn sync_stream_inserts_the_stream_key_only_without_fake_stream(
    #[case] fake_stream: bool,
    #[case] extra: Option<bool>,
    #[case] expected: Option<bool>,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Sync);
    call.fake_stream = fake_stream;
    call.options.insert("stream".to_owned(), Json::Bool(true));
    if let Some(stream) = extra {
        call.options.insert(
            "extra_body".to_owned(),
            Json::Object(IndexMap::from([("stream".to_owned(), Json::Bool(stream))])),
        );
    }
    let plan = plan(&call, &mut state, &env);
    assert_eq!(plan.body.get("stream"), expected.map(Json::Bool).as_ref());
}

#[test]
fn async_stream_writes_the_stream_flag_after_the_pre_call_log() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.fake_stream = true;
    call.options.insert("stream".to_owned(), Json::Bool(true));
    call.options.insert(
        "extra_body".to_owned(),
        Json::Object(IndexMap::from([("stream".to_owned(), Json::Bool(false))])),
    );
    let plan = plan(&call, &mut state, &env);
    assert_eq!(plan.body.get("stream"), Some(&Json::Bool(true)));
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the pre-call arguments");
    };
    let Some(Json::Object(logged)) = args.get("complete_input_dict") else {
        panic!("the logged body");
    };
    assert_eq!(logged.get("stream"), Some(&Json::Bool(false)));
    let answer = ChatAnswer::Stream {
        body: empty_body(),
        limit: 4096,
    };
    let outcome = finish_chat(&plan, answer, &mut state, &env).unwrap();
    assert!(matches!(
        outcome,
        CompatibleChatOutcome::Stream(stream) if matches!(*stream, CompatibleChatStream::Lines(_))
    ));
}

#[rstest]
#[case(Mode::Async, false, None, Some(DEFAULT_TIMEOUT), None, 600)]
#[case(Mode::Async, false, Some(30), Some(UNIFORM_30), None, 30)]
#[case(Mode::Async, true, Some(30), None, None, 6000)]
#[case(Mode::Sync, true, Some(30), Some(UNIFORM_30), None, 30)]
#[case(Mode::Sync, true, None, None, None, 600)]
#[case(Mode::Sync, false, Some(30), None, Some(UNIFORM_30), 30)]
#[case(Mode::Sync, false, None, None, None, 600)]
fn timeouts_follow_the_source_branches(
    #[case] mode: Mode,
    #[case] stream: bool,
    #[case] seconds: Option<u64>,
    #[case] send: Option<TimeoutFacts>,
    #[case] construct: Option<TimeoutFacts>,
    #[case] deadline: u64,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(mode);
    call.timeout = seconds.map(Duration::from_secs);
    call.options.insert("stream".to_owned(), Json::Bool(stream));
    let plan = plan(&call, &mut state, &env);
    assert_eq!(plan.send_timeout, send);
    assert_eq!(plan.construct_timeout, construct);
    assert_eq!(plan.deadline(), Duration::from_secs(deadline));
    let effective = send.or(construct).unwrap_or(DEFAULT_TIMEOUT);
    assert_eq!(plan.effective_timeout(), effective);
    let request = if stream && mode == Mode::Async {
        None
    } else {
        Some(effective)
    };
    assert_eq!(plan.request_timeout(), request);
    assert_eq!(plan.buffered(), !stream);
}

#[test]
fn only_the_blocking_fake_stream_reads_the_whole_body() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Sync);
    call.options.insert("stream".to_owned(), Json::Bool(true));
    call.fake_stream = true;
    assert!(plan(&call, &mut state, &env).buffered());
    call.mode = Mode::Async;
    assert!(!plan(&call, &mut state, &env).buffered());
}

impl MessageTransformer for Suffix {
    fn transform(
        &self,
        model: &str,
        mut messages: Vec<ChatMessage>,
    ) -> Result<Vec<ChatMessage>, Box<dyn std::error::Error + Send + Sync>> {
        messages[0].content = Some(MessageContent::Text(format!("{model}!")));
        Ok(messages)
    }
}

struct Refuse;

impl MessageTransformer for Refuse {
    fn transform(
        &self,
        _model: &str,
        _messages: Vec<ChatMessage>,
    ) -> Result<Vec<ChatMessage>, Box<dyn std::error::Error + Send + Sync>> {
        Err(Box::from(std::io::Error::other("no transform")))
    }
}

#[test]
fn the_selected_transform_rewrites_the_logged_and_sent_messages() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let call = Call::new(Mode::Async);
    let plan = call
        .execution(Some(&Suffix), None)
        .plan(&mut state, &env)
        .unwrap();
    assert_eq!(plan.body.get("messages"), Some(&user_message("gpt-4o!")));
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&user_message("gpt-4o!"))
    );
}

#[test]
fn without_the_capability_the_transform_never_runs() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let call = Call::new(Mode::Async);
    let plan = plan(&call, &mut state, &env);
    assert_eq!(plan.body.get("messages"), Some(&user_message("hi")));
    let refused = call.execution(Some(&Refuse), None).plan(&mut state, &env);
    assert!(matches!(refused, Err(CompatibleChatError::Transform(_))));
}

#[tokio::test]
async fn a_nonstream_reply_is_decoded_with_the_provider_prefix_and_base_model() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    call.options.insert(
        "base_model".to_owned(),
        Json::Text(String::from("upstream")),
    );
    let (request, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 201, REPLY.as_bytes()), async {
            let plan = plan(&call, &mut state, &env);
            let answer = send_chat(&client(), &plan).await.unwrap();
            finish_chat(&plan, answer, &mut state, &env)
        })
    })
    .await
    .unwrap();
    let CompatibleChatOutcome::Reply(reply) = answer.unwrap() else {
        panic!("a buffered reply");
    };
    assert_eq!(reply.response.model, "openai_like/gpt-4o");
    assert_eq!(reply.hidden_model.as_deref(), Some("upstream"));
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    assert!(matches!(
        state.model_call_details.get("original_response"),
        Some(Json::Text(text)) if text.contains("\"id\": \"chat-1\"")
    ));
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(request.contains(r#""model": "gpt-4o""#));
}

#[tokio::test]
async fn a_sync_fake_stream_buffers_and_replays_one_chunk() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Sync);
    call.api_base = Some(base(&listener).await);
    call.fake_stream = true;
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let (request, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 200, FAKE_REPLY.as_bytes()), async {
            let plan = plan(&call, &mut state, &env);
            let answer = send_chat(&client(), &plan).await.unwrap();
            finish_chat(&plan, answer, &mut state, &env)
        })
    })
    .await
    .unwrap();
    let CompatibleChatOutcome::Stream(stream) = answer.unwrap() else {
        panic!("the fake stream");
    };
    let CompatibleChatStream::Fake(mut fake) = *stream else {
        panic!("the fake stream");
    };
    let chunk = fake.next().await.unwrap().unwrap();
    assert_eq!(chunk.text, "fake");
    assert!(chunk.is_finished);
    assert_eq!(chunk.usage.unwrap().prompt_tokens, 1);
    assert!(fake.next().await.is_none());
    assert!(!request.contains(r#""stream""#));
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&Json::Text(String::from("first stream response received")))
    );
}

#[tokio::test]
async fn an_async_fake_stream_is_dropped_and_reads_lines() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    call.fake_stream = true;
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let (request, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 200, LINES.as_bytes()), async {
            let plan = plan(&call, &mut state, &env);
            let answer = send_chat(&client(), &plan).await.unwrap();
            finish_chat(&plan, answer, &mut state, &env)
        })
    })
    .await
    .unwrap();
    let CompatibleChatOutcome::Stream(stream) = answer.unwrap() else {
        panic!("the line stream");
    };
    let CompatibleChatStream::Lines(mut lines) = *stream else {
        panic!("the line stream");
    };
    let chunk = lines.next().await.unwrap().unwrap();
    assert_eq!(chunk.text, "hi");
    assert!(lines.next().await.is_none());
    assert!(request.contains(r#""stream": true"#));
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&Json::Text(String::from("stream")))
    );
}

#[tokio::test]
async fn a_sync_stream_requires_exactly_200_and_logs_the_first_response() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Sync);
    call.api_base = Some(base(&listener).await);
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let (_, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 201, LINES.as_bytes()), async {
            let plan = plan(&call, &mut state, &env);
            send_chat(&client(), &plan).await
        })
    })
    .await
    .unwrap();
    assert!(matches!(
        answer,
        Err(CompatibleChatError::OpenAiLike(error)) if error.status_code == 201
    ));
    assert_eq!(
        state.model_call_details.get("log_event_type"),
        Some(&Json::Text(String::from("pre_api_call")))
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut call = Call::new(Mode::Sync);
    call.api_base = Some(base(&listener).await);
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let (_, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 200, LINES.as_bytes()), async {
            let plan = plan(&call, &mut state, &env);
            let answer = send_chat(&client(), &plan).await.unwrap();
            finish_chat(&plan, answer, &mut state, &env)
        })
    })
    .await
    .unwrap();
    let CompatibleChatOutcome::Stream(stream) = answer.unwrap() else {
        panic!("the line stream");
    };
    let CompatibleChatStream::Lines(mut lines) = *stream else {
        panic!("the line stream");
    };
    let chunk = lines.next().await.unwrap().unwrap();
    assert_eq!(chunk.text, "hi");
    let done = lines.next().await.unwrap().unwrap();
    assert!(done.text.is_empty());
    assert!(lines.next().await.is_none());
    assert_eq!(
        state.model_call_details.get("original_response"),
        Some(&Json::Text(String::from("first stream response received")))
    );
}

#[tokio::test]
async fn an_async_stream_ignores_the_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let (_, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 404, LINES.as_bytes()), async {
            let plan = plan(&call, &mut state, &env);
            send_chat(&client(), &plan).await
        })
    })
    .await
    .unwrap();
    assert!(matches!(answer, Ok(ChatAnswer::Stream { .. })));
}

#[tokio::test]
async fn a_buffered_status_error_keeps_the_body_text() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    let (_, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 404, b"missing"), async {
            let plan = plan(&call, &mut state, &env);
            let answer = send_chat(&client(), &plan).await.unwrap();
            finish_chat(&plan, answer, &mut state, &env)
        })
    })
    .await
    .unwrap();
    assert!(matches!(
        answer,
        Err(CompatibleChatError::OpenAiLike(error)) if error.status_code == 404 && error.message == "missing"
    ));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut call = Call::new(Mode::Sync);
    call.api_base = Some(base(&listener).await);
    let (_, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 300, b"moved"), async {
            let plan = plan(&call, &mut state, &env);
            let answer = send_chat(&client(), &plan).await.unwrap();
            finish_chat(&plan, answer, &mut state, &env)
        })
    })
    .await
    .unwrap();
    assert!(matches!(
        answer,
        Err(CompatibleChatError::OpenAiLike(error)) if error.status_code == 300 && error.message == "moved"
    ));
}

#[tokio::test]
async fn a_status_body_that_is_not_utf8_is_its_own_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Sync);
    call.api_base = Some(base(&listener).await);
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let (_, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 400, &[0xff, 0xfe]), async {
            let plan = plan(&call, &mut state, &env);
            send_chat(&client(), &plan).await
        })
    })
    .await
    .unwrap();
    let Err(error) = answer else {
        panic!("a status error");
    };
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        error,
        CompatibleChatError::StatusBody { status: 400, .. }
    ));
}

#[tokio::test]
async fn a_stream_send_failure_stays_a_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let plan = plan(&call, &mut state, &env);
    drop(listener);
    let answer = timeout(EXCHANGE_TIMEOUT, send_chat(&client(), &plan))
        .await
        .unwrap();
    let Err(error) = answer else {
        panic!("the transport error");
    };
    assert!(matches!(error, CompatibleChatError::Transport(_)));
    assert_eq!(error.status_code(), Some(500));
}

#[tokio::test]
async fn a_nonstream_send_failure_is_a_timeout_or_a_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    call.timeout = Some(Duration::from_millis(20));
    let plan = plan(&call, &mut state, &env);
    let answer = timeout(EXCHANGE_TIMEOUT, send_chat(&client(), &plan))
        .await
        .unwrap();
    let Err(error) = answer else {
        panic!("the deadline error");
    };
    assert!(matches!(error, CompatibleChatError::Timeout(_)));
    assert_eq!(error.status_code(), Some(408));
    assert_eq!(error.to_string(), "Timeout error occurred.");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    let plan = plan(&call, &mut state, &env);
    drop(listener);
    let answer = timeout(EXCHANGE_TIMEOUT, send_chat(&client(), &plan))
        .await
        .unwrap();
    let Err(error) = answer else {
        panic!("the transport error");
    };
    assert!(matches!(error, CompatibleChatError::Transport(_)));
    assert_eq!(error.status_code(), Some(500));
}

struct Recording(Arc<Mutex<Vec<usize>>>);

impl StreamingDecoder for Recording {
    fn decode(&self, bytes: Bytes) -> DecodedChunks {
        let reads = Arc::clone(&self.0);
        bytes
            .map(move |read| {
                let read = read?;
                reads
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(read.len());
                Ok(empty_chunk())
            })
            .boxed()
    }
}

#[tokio::test]
async fn a_custom_decoder_wins_and_reads_1024_byte_chunks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) = logging(&redactor);
    let mut call = Call::new(Mode::Async);
    call.api_base = Some(base(&listener).await);
    call.fake_stream = true;
    call.options.insert("stream".to_owned(), Json::Bool(true));
    let body = "a".repeat(3000);
    let reads = Arc::new(Mutex::new(Vec::new()));
    let decoder = Recording(Arc::clone(&reads));
    let (_, answer) = timeout(EXCHANGE_TIMEOUT, async {
        tokio::join!(serve(&listener, 200, body.as_bytes()), async {
            let plan = call
                .execution(None, Some(&decoder))
                .plan(&mut state, &env)
                .unwrap();
            let answer = send_chat(&client(), &plan).await.unwrap();
            finish_chat(&plan, answer, &mut state, &env)
        })
    })
    .await
    .unwrap();
    let CompatibleChatOutcome::Stream(stream) = answer.unwrap() else {
        panic!("the decoder stream");
    };
    let CompatibleChatStream::Decoder(mut chunks) = *stream else {
        panic!("the decoder stream");
    };
    while chunks.next().await.is_some() {}
    assert_eq!(*reads.lock().unwrap(), vec![1024, 1024, 952]);
}
