#![cfg(test)]

use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::api::responses_session_history::SessionLookupSettings;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_streaming::stream_chunk_shaping::creation::system_instant;
use betterlite_streaming::stream_chunk_shaping::records::StreamOptions as WrapperStreamOptions;
use betterlite_streaming::streamed_response_assembly::AssemblyError;
use betterlite_streaming::streamed_response_assembly::response::Response as StreamResponseRecord;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Value;
use tracing::subscriber::{DefaultGuard, set_default};
use uuid::Uuid;

use crate::api::api_boundaries::ApiLimits;
use crate::api::bfl::poll::PollSleeper;
use crate::api::chat_response_stream::driver::ChatResponseStream;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::chat_response_stream_items::snapshot::StreamAssembly;
use crate::api::chat_response_stream_items::sources::EventSources;
use crate::api::responses_chat_output::records::AssembledResponse;
use crate::api::responses_session_history::{
    ColdStorageReader, SessionHistory, SessionLog, SpendLogQuery,
};
use crate::api::responses_tool_repair::value::Value as RepairValue;
use crate::shadow_requests::surfaces::environment::{NoBridge, NoParameters, empty_cache};

use super::completions::{CompletionBody, CompletionClient, CompletionOutput};
use super::error::CallError;
use super::handler::{async_response_handler, response_handler};
use super::records::{CallOutcome, CompletionArguments, ResponseCall, ResponseChatCall};
use super::source::{ChunkSourceConfig, ResponsesChunkSource};

const NORMALIZED_ID: &str = "msg_00000000-0000-0000-0000-000000000000";
const DELTA: &str = "data: {\"id\":\"chat_a\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n";
const FINISH: &str = "data: {\"id\":\"chat_a\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"gpt-4o-mini\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
const DONE: &str = "data: [DONE]\n\n";

struct World {
    cache: DualLocalCache,
    parameters: NoParameters,
    probe: NoBridge,
}

impl World {
    fn new() -> Self {
        Self {
            cache: empty_cache().unwrap(),
            parameters: NoParameters,
            probe: NoBridge,
        }
    }

    fn deps<'deps, Completions>(
        &'deps self,
        completions: &'deps Completions,
    ) -> ResponseChatCall<'deps, Completions> {
        ResponseChatCall {
            completions,
            limits: ApiLimits::default(),
            cache: &self.cache,
            parameters: &self.parameters,
            probe: &self.probe,
            route_all_openai_to_responses: false,
        }
    }
}

struct NoAssembly;

impl StreamAssembly for NoAssembly {
    fn assemble(
        &self,
        _chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<StreamResponseRecord>, AssemblyError> {
        Ok(None)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct FakeError(String);

enum Answer {
    Chat(Value),
    Stream(Vec<Vec<u8>>),
    Failure(String),
    Unexpected(String),
}

struct FakeCompletions {
    answer: Answer,
    calls: Mutex<Vec<IndexMap<String, Value>>>,
}

impl FakeCompletions {
    fn chat(body: Value) -> Self {
        Self {
            answer: Answer::Chat(body),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn stream(chunks: Vec<Vec<u8>>) -> Self {
        Self {
            answer: Answer::Stream(chunks),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            answer: Answer::Failure(message.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn unexpected(python_type: &str) -> Self {
        Self {
            answer: Answer::Unexpected(python_type.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<IndexMap<String, Value>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn sole_call(&self) -> IndexMap<String, Value> {
        let calls = self.calls();
        assert_eq!(calls.len(), 1);
        calls.into_iter().next().unwrap()
    }
}

impl CompletionClient for FakeCompletions {
    type Error = FakeError;

    fn completion(
        &self,
        arguments: &CompletionArguments,
    ) -> impl Future<Output = Result<CompletionOutput, Self::Error>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(arguments.fields.clone());
        let result = match &self.answer {
            Answer::Chat(body) => Ok(CompletionOutput::Response(Box::new(
                serde_json::from_value(body.clone()).unwrap(),
            ))),
            Answer::Stream(chunks) => {
                let frames = chunks.clone();
                Ok(CompletionOutput::Stream(CompletionBody(
                    futures_util::stream::iter(frames.into_iter().map(Ok::<Vec<u8>, HttpError>))
                        .boxed(),
                )))
            }
            Answer::Failure(message) => Err(FakeError(message.clone())),
            Answer::Unexpected(python_type) => Ok(CompletionOutput::Unexpected {
                python_type: python_type.clone(),
            }),
        };
        ready(result)
    }
}

struct ScriptedQuery {
    rows: Vec<SessionLog>,
    lookups: Mutex<Vec<String>>,
}

impl ScriptedQuery {
    fn with_history(messages: &str) -> Self {
        let stored = json(&format!(r#"{{"messages":{messages}}}"#));
        Self {
            rows: vec![SessionLog {
                session_id: Some("sess_a".to_owned()),
                proxy_server_request: Some(decision(&stored.to_string())),
                metadata: None,
                response: None,
                columns: IndexMap::new(),
            }],
            lookups: Mutex::new(Vec::new()),
        }
    }

    fn lookups(&self) -> Vec<String> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendLogQuery for ScriptedQuery {
    fn session_logs(
        &self,
        _sql: &'static str,
        request_id: &str,
    ) -> impl Future<Output = Result<Vec<SessionLog>, TableError>> + Send {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request_id.to_owned());
        ready(Ok(self.rows.clone()))
    }
}

struct NoCold;

impl ColdStorageReader for NoCold {
    type Error = Infallible;

    fn proxy_server_request(
        &self,
        _object_key: &str,
    ) -> impl Future<Output = Result<Option<IndexMap<String, Json>>, Infallible>> + Send {
        ready(Ok(None))
    }
}

struct NoSleep;

impl PollSleeper for NoSleep {
    fn sleep(&self, _duration: Duration) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

fn history<'deps>(
    query: &'deps ScriptedQuery,
    sleeper: &'deps NoSleep,
    cache: &'deps DualLocalCache,
    tool_calls: &'deps IndexMap<String, RepairValue>,
) -> SessionHistory<'deps, ScriptedQuery, NoCold, NoSleep> {
    SessionHistory {
        repository: Some(query),
        cold_storage: None,
        sleeper,
        lookup: SessionLookupSettings {
            max_attempts: 1_u64,
            retry_interval_seconds: 0.2_f64,
        },
        disable_spend_logs: true,
        limits: ApiLimits::default(),
        input_cache: cache,
        tool_calls,
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

fn json(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn decision(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn params(source: &str) -> ResponsesApiRequestParams {
    serde_json::from_str(source).unwrap()
}

fn kwargs(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).unwrap()
}

fn sources() -> EventSources {
    EventSources {
        clock: Arc::new(|| Ok(0_i64)),
        ids: Arc::new(|| NORMALIZED_ID.to_owned()),
    }
}

fn call<'call>(
    model: &'call str,
    request: &'call ResponsesApiRequestParams,
    caller: &'call IndexMap<String, Json>,
) -> ResponseCall<'call, NoAssembly> {
    ResponseCall {
        model,
        request,
        custom_llm_provider: None,
        stream: None,
        extra_headers: None,
        kwargs: caller,
        logging: None,
        sources: sources(),
        context: NoAssembly,
    }
}

fn chat_response(id: &str, content: &str) -> Value {
    json(&format!(
        r#"{{"id":"{id}","created":0,"model":"gpt-4o-mini","choices":[{{"index":0,"message":{{"role":"assistant","content":"{content}"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}}}"#
    ))
}

fn chat_tool_call(id: &str, name: &str) -> Value {
    json(&format!(
        r#"{{"id":"{id}","created":0,"model":"gpt-4o-mini","choices":[{{"index":0,"message":{{"role":"assistant","content":null,"tool_calls":[{{"id":"call_1","type":"function","function":{{"name":"{name}","arguments":"{{}}"}}}}]}},"finish_reason":"tool_calls"}}],"usage":{{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}}}"#
    ))
}

fn response_of(outcome: CallOutcome<NoAssembly>) -> AssembledResponse {
    let CallOutcome::Response(assembled) = outcome else {
        panic!("a chat answer converts to a Responses object");
    };
    *assembled
}

fn stream_of(
    outcome: CallOutcome<NoAssembly>,
) -> ChatResponseStream<ResponsesChunkSource, NoAssembly> {
    let CallOutcome::Stream(stream) = outcome else {
        panic!("a provider stream becomes the Responses iterator");
    };
    stream
}

fn messages_of(arguments: &IndexMap<String, Value>) -> Vec<Value> {
    arguments
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn chat_request_precedence() {
    let completions = FakeCompletions::chat(chat_response("chat_a", "hi"));
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(r#"{"model":"gpt-4o-mini","input":"hello","max_output_tokens":7}"#);
    let extra_headers = decision(r#"{"x-client":"probe"}"#);
    let caller =
        kwargs(r#"{"max_tokens":3,"api_key":"sk-test","_skip_responses_api_bridge":false}"#);
    let mut call = call("gpt-4o-mini", &request, &caller);
    call.extra_headers = Some(&extra_headers);
    let outcome = response_handler(&deps, call).await.unwrap();
    let arguments = completions.sole_call();
    assert_eq!(arguments.get("max_tokens"), Some(&Value::from(7_i64)));
    assert_eq!(arguments.get("model"), Some(&Value::from("gpt-4o-mini")));
    assert_eq!(arguments.get("api_key"), Some(&Value::from("sk-test")));
    assert_eq!(
        arguments.get("_skip_responses_api_bridge"),
        Some(&Value::from(true))
    );
    assert_eq!(
        arguments.get("extra_headers"),
        Some(&json(r#"{"x-client":"probe"}"#))
    );
    assert!(arguments.get("input").is_none());
    assert!(arguments.get("max_output_tokens").is_none());
    assert_eq!(
        messages_of(&arguments),
        vec![json(r#"{"role":"user","content":"hello"}"#)]
    );
    let encoded = serde_json::to_value(&response_of(outcome).response).unwrap();
    assert_eq!(encoded.get("id"), Some(&Value::from("chat_a")));
    assert_eq!(encoded.get("object"), Some(&Value::from("response")));
    assert_eq!(encoded["output"][0]["type"], json("\"message\""));
    assert_eq!(encoded["output"][0]["content"][0]["text"], json("\"hi\""));
}

const HOISTED_INPUT: &str = r#"[
    {"role":"user","content":"hello"},
    {"type":"additional_tools","role":"developer","tools":[
        {"type":"function","name":"lookup","description":"Find","parameters":{"type":"object","properties":{}}}
    ]}
]"#;

const CUSTOM_HOISTED_INPUT: &str = r#"[
    {"role":"user","content":"hello"},
    {"type":"additional_tools","role":"developer","tools":[
        {"type":"custom","name":"apply_patch","description":"Patch files"}
    ]}
]"#;

#[tokio::test]
async fn hoisted_tools_reach_provider() {
    let (captured, _guard) = capture_debug();
    let completions = FakeCompletions::chat(chat_tool_call("chat_a", "lookup"));
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(&format!(
        r#"{{"model":"gpt-4o-mini","input":{HOISTED_INPUT}}}"#
    ));
    let caller = IndexMap::new();
    let outcome = response_handler(&deps, call("gpt-4o-mini", &request, &caller))
        .await
        .unwrap();
    let arguments = completions.sole_call();
    assert_eq!(
        messages_of(&arguments),
        vec![json(r#"{"role":"user","content":"hello"}"#)]
    );
    let tools = arguments
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["function"]["name"], json("\"lookup\""));
    assert!(
        captured
            .text()
            .contains("hoisting 1 tool(s) out of 1 'additional_tools'")
    );
    let encoded = serde_json::to_value(&response_of(outcome).response).unwrap();
    assert_eq!(encoded["output"][0]["type"], json("\"function_call\""));
}

#[tokio::test]
async fn hoisted_custom_tool_shapes_output() {
    let completions = FakeCompletions::chat(chat_tool_call("chat_a", "apply_patch"));
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(&format!(
        r#"{{"model":"gpt-4o-mini","input":{CUSTOM_HOISTED_INPUT}}}"#
    ));
    let caller = IndexMap::new();
    let outcome = response_handler(&deps, call("gpt-4o-mini", &request, &caller))
        .await
        .unwrap();
    let encoded = serde_json::to_value(&response_of(outcome).response).unwrap();
    assert_eq!(encoded["output"][0]["type"], json("\"custom_tool_call\""));
}

#[rstest]
#[case::explicit_null(r#"{"model":"m","input":"hello","tools":null}"#)]
#[case::empty_list(r#"{"model":"m","input":"hello","tools":[]}"#)]
#[tokio::test]
async fn absent_tools_pass_the_request_through(#[case] source: &str) {
    let completions = FakeCompletions::chat(chat_response("chat_a", "hi"));
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(source);
    let caller = IndexMap::new();
    let outcome = response_handler(&deps, call("m", &request, &caller))
        .await
        .unwrap();
    let arguments = completions.sole_call();
    assert_eq!(
        messages_of(&arguments),
        vec![json(r#"{"role":"user","content":"hello"}"#)]
    );
    assert!(arguments.get("tools").is_none());
    assert_eq!(response_of(outcome).response.output.len(), 1);
}

#[tokio::test]
async fn async_session_only() {
    let completions = FakeCompletions::chat(chat_response("chat_a", "hi"));
    let world = World::new();
    let deps = world.deps(&completions);
    let query = ScriptedQuery::with_history(r#"[{"role":"user","content":"old"}]"#);
    let sleeper = NoSleep;
    let tool_calls = IndexMap::new();
    let session = history(&query, &sleeper, &world.cache, &tool_calls);
    let request =
        params(r#"{"model":"gpt-4o-mini","input":"hello","previous_response_id":"resp_a"}"#);
    let caller = IndexMap::new();
    async_response_handler(&deps, &session, call("gpt-4o-mini", &request, &caller))
        .await
        .unwrap();
    let arguments = completions.sole_call();
    let messages = messages_of(&arguments);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"], json("\"old\""));
    assert_eq!(messages[1]["content"], json("\"hello\""));
    assert_eq!(
        arguments.get("litellm_trace_id"),
        Some(&Value::from("sess_a"))
    );
    assert_eq!(query.lookups(), vec!["resp_a".to_owned()]);
}

#[tokio::test]
async fn sync_never_looks_up_the_session() {
    let completions = FakeCompletions::chat(chat_response("chat_a", "hi"));
    let world = World::new();
    let deps = world.deps(&completions);
    let query = ScriptedQuery::with_history(r#"[{"role":"user","content":"old"}]"#);
    let request =
        params(r#"{"model":"gpt-4o-mini","input":"hello","previous_response_id":"resp_a"}"#);
    let caller = IndexMap::new();
    response_handler(&deps, call("gpt-4o-mini", &request, &caller))
        .await
        .unwrap();
    let arguments = completions.sole_call();
    let messages = messages_of(&arguments);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], json("\"hello\""));
    assert!(arguments.get("litellm_trace_id").is_none());
    assert!(query.lookups().is_empty());
}

#[rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
#[tokio::test]
async fn empty_previous_response_id_is_no_lookup(#[case] asynchronous: bool) {
    let completions = FakeCompletions::chat(chat_response("chat_a", "hi"));
    let world = World::new();
    let deps = world.deps(&completions);
    let query = ScriptedQuery::with_history(r#"[{"role":"user","content":"old"}]"#);
    let sleeper = NoSleep;
    let tool_calls = IndexMap::new();
    let session = history(&query, &sleeper, &world.cache, &tool_calls);
    let request = params(r#"{"model":"gpt-4o-mini","input":"hello","previous_response_id":""}"#);
    let caller = IndexMap::new();
    let call = call("gpt-4o-mini", &request, &caller);
    if asynchronous {
        async_response_handler(&deps, &session, call).await.unwrap();
    } else {
        response_handler(&deps, call).await.unwrap();
    }
    assert_eq!(messages_of(&completions.sole_call()).len(), 1);
    assert!(completions.sole_call().get("litellm_trace_id").is_none());
    assert!(query.lookups().is_empty());
}

fn stream_chunks() -> Vec<Vec<u8>> {
    vec![
        DELTA.as_bytes().to_vec(),
        format!("{FINISH}{DONE}").into_bytes(),
    ]
}

#[tokio::test]
async fn chat_stream_dispatch() {
    let completions = FakeCompletions::stream(stream_chunks());
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(r#"{"model":"gpt-4o-mini","input":"hi","stream":true}"#);
    let caller = kwargs(r#"{"litellm_metadata":{"trace":"t"}}"#);
    let mut call = call("handler-model", &request, &caller);
    call.custom_llm_provider = Some("openai");
    call.stream = Some(true);
    let mut stream = stream_of(response_handler(&deps, call).await.unwrap());
    assert_eq!(stream.state.model, "handler-model");
    assert_eq!(stream.state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(stream.state.request_input, Json::Text("hi".to_owned()));
    let Json::Object(mapped) = &stream.state.responses_api_request else {
        panic!("the mapped request is an object");
    };
    assert_eq!(
        mapped.get("model"),
        Some(&Json::Text("gpt-4o-mini".to_owned()))
    );
    assert_eq!(mapped.get("input"), Some(&Json::Text("hi".to_owned())));
    assert!(mapped.get("_skip_responses_api_bridge").is_none());
    assert_eq!(
        stream.state.litellm_metadata.get("trace"),
        Some(&Json::Text("t".to_owned()))
    );
    assert_eq!(
        completions.sole_call().get("stream_options"),
        Some(&json(r#"{"include_usage":true}"#))
    );
    let mut events: usize = 0;
    while stream.next_event_async().await.unwrap().is_some() {
        events += 1;
    }
    assert!(events > 0);
}

fn chunk_source(frames: Vec<Vec<u8>>) -> ResponsesChunkSource {
    let body = CompletionBody(
        futures_util::stream::iter(frames.into_iter().map(Ok::<Vec<u8>, HttpError>)).boxed(),
    );
    ResponsesChunkSource::new(
        ChunkSourceConfig {
            provider: "openai".to_owned(),
            model: "gpt-4o-mini".to_owned(),
            stream_options: Some(WrapperStreamOptions {
                include_usage: true,
            }),
            now: system_instant,
            next_id: Uuid::new_v4,
        },
        body,
    )
}

async fn drained(source: &mut ResponsesChunkSource) -> Vec<Value> {
    let mut chunks = Vec::new();
    while let Some(chunk) = source.next_chunk().await.unwrap() {
        chunks.push(serde_json::to_value(&chunk).unwrap());
    }
    chunks
}

#[tokio::test]
async fn source_chunks_end_with_the_usage_chunk() {
    let mut source = chunk_source(stream_chunks());
    let chunks = drained(&mut source).await;
    assert_eq!(chunks[0]["choices"][0]["delta"]["content"], json("\"hi\""));
    assert_eq!(chunks[1]["choices"][0]["finish_reason"], json("\"stop\""));
    assert!(
        chunks.iter().any(|chunk| !chunk["usage"].is_null()),
        "the wrapper's closing usage chunk goes out: {chunks:?}"
    );
}

#[tokio::test]
async fn source_stops_reading_after_done() {
    let mut frames = stream_chunks();
    frames.push(b"data: {broken\n\n".to_vec());
    let mut source = chunk_source(frames);
    let chunks = drained(&mut source).await;
    assert!(chunks.iter().any(|chunk| !chunk["usage"].is_null()));
}

#[rstest]
#[case::sync(false, "handler-model")]
#[case::asynchronous(true, "mapped-model")]
#[tokio::test]
async fn stream_identity_follows_the_entry_mode(
    #[case] asynchronous: bool,
    #[case] expected: &str,
) {
    let completions = FakeCompletions::stream(stream_chunks());
    let world = World::new();
    let deps = world.deps(&completions);
    let query = ScriptedQuery::with_history("[]");
    let sleeper = NoSleep;
    let tool_calls = IndexMap::new();
    let session = history(&query, &sleeper, &world.cache, &tool_calls);
    let request = params(r#"{"model":"mapped-model","input":"hi","stream":true}"#);
    let caller = IndexMap::new();
    let mut call = call("handler-model", &request, &caller);
    call.custom_llm_provider = Some("openai");
    call.stream = Some(true);
    let outcome = if asynchronous {
        async_response_handler(&deps, &session, call).await.unwrap()
    } else {
        response_handler(&deps, call).await.unwrap()
    };
    assert_eq!(stream_of(outcome).state.model, expected);
}

#[tokio::test]
async fn async_stream_names_an_empty_model_when_falsy() {
    let completions = FakeCompletions::stream(stream_chunks());
    let world = World::new();
    let deps = world.deps(&completions);
    let query = ScriptedQuery::with_history("[]");
    let sleeper = NoSleep;
    let tool_calls = IndexMap::new();
    let session = history(&query, &sleeper, &world.cache, &tool_calls);
    let request = params(r#"{"model":"","input":"hi","stream":true}"#);
    let caller = IndexMap::new();
    let mut call = call("handler-model", &request, &caller);
    call.custom_llm_provider = Some("openai");
    call.stream = Some(true);
    let outcome = async_response_handler(&deps, &session, call).await.unwrap();
    assert_eq!(stream_of(outcome).state.model, "");
}

#[tokio::test]
async fn chat_provider_failure() {
    let completions = FakeCompletions::failing("bad key");
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(r#"{"model":"gpt-4o-mini","input":"hello"}"#);
    let caller = IndexMap::new();
    let Err(error) = response_handler(&deps, call("gpt-4o-mini", &request, &caller)).await else {
        panic!("a provider failure is not converted into a Responses success");
    };
    assert!(matches!(error, CallError::Completion(_)));
    assert_eq!(error.to_string(), "bad key");
    assert_eq!(completions.calls().len(), 1);
}

#[tokio::test]
async fn unexpected_executor_answer_is_a_value_error() {
    let completions = FakeCompletions::unexpected("str");
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(r#"{"model":"gpt-4o-mini","input":"hello"}"#);
    let caller = IndexMap::new();
    let Err(error) = response_handler(&deps, call("gpt-4o-mini", &request, &caller)).await else {
        panic!("an impossible executor answer is a conversion failure");
    };
    assert!(matches!(error, CallError::Conversion(_)));
    assert_eq!(error.to_string(), "Unexpected response type: <class 'str'>");
}

#[rstest]
#[case::service_tier(r#"{"service_tier":5}"#, "service_tier")]
#[case::litellm_metadata(r#"{"litellm_metadata":5}"#, "litellm_metadata")]
#[tokio::test]
async fn wrong_kwarg_types_are_refused(#[case] source: &str, #[case] field: &str) {
    let completions = FakeCompletions::chat(chat_response("chat_a", "hi"));
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(r#"{"model":"gpt-4o-mini","input":"hello"}"#);
    let caller = kwargs(source);
    let Err(error) = response_handler(&deps, call("gpt-4o-mini", &request, &caller)).await else {
        panic!("a wrongly typed kwarg is refused before the completion call");
    };
    assert!(matches!(error, CallError::Preparation(_)));
    assert!(error.to_string().contains(field));
    assert!(completions.calls().is_empty());
}

#[tokio::test]
async fn tools_must_be_a_list() {
    let completions = FakeCompletions::chat(chat_response("chat_a", "hi"));
    let world = World::new();
    let deps = world.deps(&completions);
    let request = params(r#"{"model":"gpt-4o-mini","input":"hello","tools":{}}"#);
    let caller = IndexMap::new();
    let Err(error) = response_handler(&deps, call("gpt-4o-mini", &request, &caller)).await else {
        panic!("tools that are not a list are refused before the completion call");
    };
    assert!(matches!(error, CallError::Preparation(_)));
    assert!(error.to_string().contains("tools"));
    assert!(completions.calls().is_empty());
}
