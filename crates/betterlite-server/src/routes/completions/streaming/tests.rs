#![cfg(test)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::body::to_bytes;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_core::message_utilities::response_streams::StreamError;
use betterlite_core::operation::text_stream::TextStreamResult;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::text_completion_stream::{DONE_FRAME, without_nulls};
use futures_util::{StreamExt as _, stream};
use http::HeaderMap;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::{StreamUsage, provider_stream, render};
use crate::routes::completions::bridge::chat_stream_answer;
use crate::server::chat_setup::chat_executor;

fn deployment() -> ChatDeployment {
    let listed = ModelDeployment {
        model_name: String::from("text-model"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(String::from("openai/gpt-3.5-turbo-instruct")),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
            api_key: Some(SecretString::from("configured-key")),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    };
    ChatDeployment::resolve(&listed, &OpenAiDefaults::default()).expect("deployment")
}

fn chunk(text: &str) -> Json {
    serde_json::from_str(text).expect("chunk")
}

fn result(items: Vec<Result<Json, StreamError>>) -> TextStreamResult {
    TextStreamResult {
        body: stream::iter(items).boxed(),
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::from_millis(2),
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    NullsDropped,
    DoneLast,
    ErrorTruncates,
    StreamHeaders,
    Incremental,
}

#[rstest]
#[case::nulls_dropped(Scenario::NullsDropped)]
#[case::done_last(Scenario::DoneLast)]
#[case::error_truncates(Scenario::ErrorTruncates)]
#[case::stream_headers(Scenario::StreamHeaders)]
#[case::incremental(Scenario::Incremental)]
#[tokio::test]
async fn text_streaming_text_stream_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::NullsDropped => nulls_dropped().await,
        Scenario::DoneLast => done_last().await,
        Scenario::ErrorTruncates => error_truncates().await,
        Scenario::StreamHeaders => stream_headers(),
        Scenario::Incremental => incremental().await,
    }
}

async fn nulls_dropped() {
    let answer = result(vec![Ok(chunk(
        r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"text-model","choices":[{"finish_reason":null,"index":0,"text":" Hello","logprobs":null}],"usage":{"completion_tokens":0,"prompt_tokens":0,"total_tokens":0}}"#,
    ))]);
    let response = render(answer, &deployment(), "call", Instant::now());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8(body.to_vec()).expect("utf8");
    assert!(text.starts_with("data: "), "{text}");
    let event_text = text.strip_suffix(DONE_FRAME).expect("done frame");
    let event: Json =
        serde_json::from_str(event_text.trim_start_matches("data: ").trim_end()).expect("event");
    assert_eq!(
        event,
        chunk(
            r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"text-model","choices":[{"index":0,"text":" Hello"}],"usage":{"completion_tokens":0,"prompt_tokens":0,"total_tokens":0}}"#
        )
    );
}

async fn done_last() {
    let response = render(result(Vec::new()), &deployment(), "call", Instant::now());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    // the whole body of an empty stream is the closing frame the route takes from DONE_FRAME
    assert_eq!(String::from_utf8(body.to_vec()).expect("utf8"), DONE_FRAME);
}

async fn error_truncates() {
    let answer = result(vec![Err(StreamError::MissingChunk)]);
    let response = render(answer, &deployment(), "call", Instant::now());
    to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_err();
}

// a chunk's frame leaves the route as soon as the provider produces it, not when the stream ends
async fn incremental() {
    let (release, released) = oneshot::channel::<Result<Json, StreamError>>();
    let first = r#"{"choices":[{"index":0,"text":" Hello"}],"created":1,"id":"cmpl-1","model":"text-model","object":"text_completion"}"#;
    let second = r#"{"choices":[{"index":0,"text":" there"}],"created":2,"id":"cmpl-2","model":"text-model","object":"text_completion"}"#;
    let body = stream::once(async { Ok(chunk(first)) })
        .chain(stream::once(async move {
            released.await.expect("the provider's second chunk")
        }))
        .boxed();
    let answer = TextStreamResult {
        body,
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::from_millis(2),
    };
    let response = render(answer, &deployment(), "call", Instant::now());
    let mut frames = response.into_body().into_data_stream();
    let first_bytes = timeout(Duration::from_secs(1), frames.next())
        .await
        .expect("the first frame waits for the provider's second chunk")
        .expect("frame")
        .expect("frame bytes");
    let first_text = String::from_utf8(first_bytes.to_vec()).expect("utf8");
    assert!(first_text.contains(r#""text":" Hello""#), "{first_text}");
    release.send(Ok(chunk(second))).expect("second chunk");
    let second_bytes = frames.next().await.expect("second frame").expect("bytes");
    let second_text = String::from_utf8(second_bytes.to_vec()).expect("utf8");
    assert!(second_text.contains(r#""text":" there""#), "{second_text}");
    let done = frames.next().await.expect("done frame").expect("bytes");
    assert_eq!(&*done, DONE_FRAME.as_bytes());
    assert!(frames.next().await.is_none());
}

fn stream_headers() {
    let response = render(result(Vec::new()), &deployment(), "call", Instant::now());
    let headers = response.headers();
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream; charset=utf-8")
    );
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-cache")
    );
    assert_eq!(
        headers
            .get("x-accel-buffering")
            .and_then(|value| value.to_str().ok()),
        Some("no")
    );
}

#[test]
fn the_null_drop_keeps_a_null_inside_a_list() {
    let cleaned = without_nulls(
        serde_json::to_value(chunk(
            r#"{"a":null,"b":[null,{"c":null,"d":1}],"e":"keep"}"#,
        ))
        .expect("value"),
    );
    let expected =
        serde_json::to_value(chunk(r#"{"b":[null,{"d":1}],"e":"keep"}"#)).expect("value");
    assert_eq!(cleaned, expected);
}

// the chat executor and its one deployment, pointed at the fake provider at this address
fn chat_setup(address: SocketAddr) -> ChatExecutor {
    let source = format!(
        "model_list:\n  - {{model_name: chat-model, litellm_params: {{model: openai/gpt-4o, api_base: 'http://{address}/v1', api_key: k}}}}\n"
    );
    let environment = BTreeMap::new();
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source)
    .expect("config");
    chat_executor(&config, &environment, StdRng::seed_from_u64(3)).expect("executor")
}

// the provider's body for one stream request, read before it answers with an empty event stream
async fn provider_body(usage: StreamUsage, request: &str) -> Json {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let executor = chat_setup(address);
    let deployment = executor.deployments[0].clone();
    let parsed = ChatRequest::parse(request.as_bytes()).expect("chat request");
    let mut tasks = JoinSet::new();
    tasks.spawn(captured_request(listener));
    let upstream = provider_stream(
        &executor,
        &deployment,
        &parsed,
        ControlOptions::default(),
        usage,
    )
    .await
    .expect("provider stream");
    drop(upstream);
    tasks
        .join_next()
        .await
        .expect("provider task")
        .expect("captured request")
}

// the request head and body a fake provider reads before it answers
async fn read_request(socket: &mut TcpStream) -> Vec<u8> {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(socket.read_u8().await.expect("request head"));
    }
    let head = String::from_utf8(received).expect("head text");
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .expect("content length")
        .parse()
        .expect("numeric length");
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.expect("request body");
    body
}

async fn captured_request(listener: TcpListener) -> Json {
    let (mut socket, _) = listener.accept().await.expect("accepted");
    let body = read_request(&mut socket).await;
    // an empty event stream: the provider decodes a stream and the test reads only the request
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 0\r\n\r\n",
        )
        .await
        .expect("response head");
    serde_json::from_slice(&body).expect("request body json")
}

// the chat provider's two events, the last one left in the parser's buffer until the stream ends
async fn chat_events(listener: TcpListener) {
    let (mut socket, _) = listener.accept().await.expect("accepted");
    let _request = read_request(&mut socket).await;
    let events = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"up-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"up-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"world\"},\"finish_reason\":null}]}\n"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{events}",
        events.len()
    );
    socket
        .write_all(response.as_bytes())
        .await
        .expect("response");
}

// the chat provider's events reach the text route as the chunks the client reads, restamped with the caller's model
#[tokio::test]
async fn chat_stream_answer_decodes_the_chat_events() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let executor = chat_setup(address);
    let deployment = executor.deployments[0].clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(chat_events(listener));
    let parsed = ChatRequest::parse(
        br#"{"model":"chat-model","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
    )
    .expect("chat request");
    let upstream = provider_stream(
        &executor,
        &deployment,
        &parsed,
        ControlOptions::default(),
        StreamUsage::Text,
    )
    .await
    .expect("provider stream");
    let answer = chat_stream_answer(upstream, Some("client"), false, Duration::ZERO);
    let items = timeout(Duration::from_secs(5), answer.body.collect::<Vec<_>>())
        .await
        .expect("the stream ends within five seconds");
    let chunks: Vec<Json> = items.into_iter().collect::<Result<_, _>>().expect("chunks");
    assert_eq!(chunks.len(), 2, "{chunks:?}");
    assert_eq!(text_field(&chunks[0], "model"), Some("client"));
    assert_eq!(chunk_content(&chunks[0]), Some("hello"));
    assert_eq!(chunk_content(&chunks[1]), Some("world"));
    tasks
        .join_next()
        .await
        .expect("provider task")
        .expect("provider");
}

fn text_field<'json>(body: &'json Json, name: &str) -> Option<&'json str> {
    let Json::Object(fields) = body else {
        return None;
    };
    let Some(Json::Text(text)) = fields.get(name) else {
        return None;
    };
    Some(text)
}

// the content of a streamed chat chunk's first choice
fn chunk_content(chunk: &Json) -> Option<&str> {
    let Json::Object(fields) = chunk else {
        return None;
    };
    let Some(Json::List(choices)) = fields.get("choices") else {
        return None;
    };
    let Some(Json::Object(choice)) = choices.first() else {
        return None;
    };
    let Some(Json::Object(delta)) = choice.get("delta") else {
        return None;
    };
    let Some(Json::Text(text)) = delta.get("content") else {
        return None;
    };
    Some(text)
}

fn include_usage(body: &Json) -> Option<bool> {
    let Json::Object(fields) = body else {
        return None;
    };
    let Some(Json::Object(options)) = fields.get("stream_options") else {
        return None;
    };
    let Some(Json::Bool(chosen)) = options.get("include_usage") else {
        return None;
    };
    Some(*chosen)
}

// atext_completion sends the caller's own stream options only; the chat route's injected usage option is not its own
#[tokio::test]
async fn a_bridged_completion_stream_keeps_no_injected_usage_options() {
    let unasked = provider_body(
        StreamUsage::Text,
        r#"{"model":"chat-model","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
    )
    .await;
    assert_eq!(text_field(&unasked, "model"), Some("gpt-4o"));
    assert_eq!(include_usage(&unasked), None, "{unasked:?}");
    let chat = provider_body(
        StreamUsage::Chat,
        r#"{"model":"chat-model","messages":[{"role":"user","content":"hi"}],"stream":true}"#,
    )
    .await;
    assert_eq!(include_usage(&chat), Some(true), "{chat:?}");
    let chosen = provider_body(
        StreamUsage::Text,
        r#"{"model":"chat-model","messages":[{"role":"user","content":"hi"}],"stream":true,"stream_options":{"include_usage":true}}"#,
    )
    .await;
    assert_eq!(include_usage(&chosen), Some(true), "{chosen:?}");
}
