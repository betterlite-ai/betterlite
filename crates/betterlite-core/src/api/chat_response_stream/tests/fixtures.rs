use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue};
use betterlite_streaming::stream_content_assembly::wire::ThinkingBlock;
use betterlite_streaming::stream_tool_assembly::records::{
    FunctionOutput, JsonValue as ToolJsonValue, ToolOutput,
};
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::AssemblyError;
use betterlite_streaming::streamed_response_assembly::response::{
    ChatChoice, ChatMessage, ChatResponse, Response,
};
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_items::snapshot::StreamAssembly;
use crate::api::chat_response_stream_items::sources::EventSources;
use crate::api::chat_response_stream_tools::state::StreamSetup;
use crate::api::messages_stream_split::errors::StreamFailure;

use super::super::driver::ChatResponseStream;
use super::super::source::{AsyncChunkStream, ChunkStream};

pub(super) type Received = Arc<Mutex<Vec<Vec<IndexMap<String, Json>>>>>;
pub(super) type SyncStream = ChatResponseStream<Scripted, FixtureAssembly>;
pub(super) type Fixture<C> = (ChatResponseStream<C, FixtureAssembly>, Received);

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(super) fn field(value: &Json, name: &str) -> Json {
    match value {
        Json::Object(fields) => fields.get(name).cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    }
}

pub(super) fn sequence(event: &Json) -> Option<i64> {
    match field(event, "sequence_number") {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn sources(ids: &[&str]) -> EventSources {
    let queue: Arc<Mutex<VecDeque<String>>> =
        Arc::new(Mutex::new(ids.iter().map(|id| (*id).to_owned()).collect()));
    let next = Arc::clone(&queue);
    EventSources {
        clock: Arc::new(|| Ok::<i64, StreamItemsError>(1_700_000_000)),
        ids: Arc::new(move || {
            next.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or_default()
        }),
    }
}

pub(super) struct Scripted {
    pub(super) chunks: VecDeque<Result<Json, StreamFailure>>,
    pub(super) pulls: Arc<AtomicUsize>,
    pub(super) released: Arc<AtomicBool>,
}

impl Scripted {
    pub(super) fn new(chunks: Vec<Result<Json, StreamFailure>>) -> Self {
        Self {
            chunks: VecDeque::from(chunks),
            pulls: Arc::new(AtomicUsize::new(0)),
            released: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn from_texts(texts: &[&str]) -> Self {
        Self::new(texts.iter().map(|text| Ok(json(text))).collect())
    }
}

impl Drop for Scripted {
    fn drop(&mut self) {
        self.released.store(true, Ordering::SeqCst);
    }
}

impl ChunkStream for Scripted {
    fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        self.chunks
            .pop_front()
            .map_or(Ok(None), |item| item.map(Some))
    }
}

// The same script drives both iterators, so a pull count means the same on either side
#[async_trait]
impl AsyncChunkStream for Scripted {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        <Self as ChunkStream>::next_chunk(self)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Reply {
    None,
    Text,
    Length,
    Signed,
    FloatTool,
    CodeInterpreter,
    ToolCall,
}

pub(super) struct FixtureAssembly {
    pub(super) reply: Reply,
    pub(super) received: Received,
}

impl FixtureAssembly {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl StreamAssembly for FixtureAssembly {
    fn assemble(
        &self,
        chunks: &[IndexMap<String, Json>],
    ) -> Result<Option<Response>, AssemblyError> {
        self.received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(chunks.to_vec());
        Ok(match self.reply {
            Reply::None => None,
            Reply::Text => Some(chat("stored", "stop", None)),
            Reply::Length => Some(chat("stored", "length", None)),
            Reply::Signed => Some(signed_reply()),
            Reply::FloatTool => Some(float_tool_reply()),
            Reply::CodeInterpreter => Some(code_interpreter_reply()),
            Reply::ToolCall => Some(chat(
                "",
                "tool_calls",
                Some(("call_1", "lookup", r#"{"x":1}"#)),
            )),
        })
    }
}

fn signed_reply() -> Response {
    let mut response = chat("", "stop", None);
    message_of(&mut response).thinking_blocks = Some(vec![ThinkingBlock::Thinking {
        thinking: String::from("t"),
        signature: String::from("sig"),
    }]);
    response
}

fn float_tool_reply() -> Response {
    let mut response = chat("", "tool_calls", Some(("call_1", "lookup", r#"{"x":1}"#)));
    let tool = &mut message_of(&mut response)
        .tool_calls
        .as_mut()
        .expect("the fixture reply carries one tool call")[0];
    tool.provider_specific_fields = Some(BTreeMap::from([(
        "score".to_owned(),
        ToolJsonValue::Number(serde_json::Number::from_f64(0.5).expect("0.5 is finite")),
    )]));
    response
}

fn code_interpreter_reply() -> Response {
    let mut response = chat("", "tool_calls", Some(("call_1", "lookup", r#"{"x":1}"#)));
    let result = JsonObject::from([
        (
            "type".to_owned(),
            JsonValue::String("code_interpreter_call".to_owned()),
        ),
        ("id".to_owned(), JsonValue::String("call_1".to_owned())),
        (
            "status".to_owned(),
            JsonValue::String("completed".to_owned()),
        ),
        ("code".to_owned(), JsonValue::String("x".to_owned())),
        (
            "container_id".to_owned(),
            JsonValue::String("ct".to_owned()),
        ),
        ("outputs".to_owned(), JsonValue::Null),
    ]);
    message_of(&mut response).provider_specific_fields = Some(JsonObject::from([(
        "code_interpreter_results".to_owned(),
        JsonValue::Array(vec![JsonValue::Object(result)]),
    )]));
    response
}

fn message_of(response: &mut Response) -> &mut ChatMessage {
    let Response::Chat(chat) = response else {
        panic!("the fixture builds a chat response");
    };
    &mut chat.choices[0].message
}

fn chat(content: &str, finish: &str, tool: Option<(&str, &str, &str)>) -> Response {
    let message = ChatMessage {
        content: Some(content.to_owned()),
        role: String::from("assistant"),
        tool_calls: tool.map(|(id, name, arguments)| {
            vec![ToolOutput {
                id: id.to_owned(),
                kind: String::from("function"),
                function: Some(FunctionOutput {
                    name: Some(name.to_owned()),
                    arguments: arguments.to_owned(),
                }),
                custom: None,
                provider_specific_fields: None,
            }]
        }),
        function_call: None,
        provider_specific_fields: None,
        reasoning_content: None,
        thinking_blocks: None,
        annotations: None,
        audio: None,
        images: None,
    };
    Response::Chat(Box::new(ChatResponse {
        id: String::from("chatcmpl-1"),
        object: String::from("chat.completion"),
        created: 0,
        model: String::from("gpt-4o-mini"),
        system_fingerprint: None,
        service_tier: None,
        choices: vec![ChatChoice {
            index: 0,
            message,
            finish_reason: finish.to_owned(),
            provider_specific_fields: None,
        }],
        usage: Usage::default(),
        hidden_params: JsonObject::new(),
    }))
}

pub(super) fn build<C>(
    source: C,
    reply: Reply,
    request: &str,
    ids: &[&str],
    litellm_metadata: Option<IndexMap<String, Json>>,
) -> (ChatResponseStream<C, FixtureAssembly>, Received) {
    let assembly = FixtureAssembly::new(reply);
    let received = Arc::clone(&assembly.received);
    let parsed = json(request);
    let stream = ChatResponseStream::new(
        StreamSetup {
            model: "gpt-4o-mini",
            logging_context: assembly,
            request_input: &Json::Text(String::from("hi")),
            responses_api_request: &parsed,
            custom_llm_provider: Some("openai"),
            litellm_metadata,
            limits: ApiLimits::default(),
        },
        sources(ids),
        source,
    )
    .expect("the driver builds");
    (stream, received)
}

pub(super) fn driver(
    chunks: Vec<Result<Json, StreamFailure>>,
    reply: Reply,
    request: &str,
    ids: &[&str],
) -> Fixture<Scripted> {
    build(Scripted::new(chunks), reply, request, ids, None)
}

pub(super) fn driver_with_source(
    source: Scripted,
    reply: Reply,
    request: &str,
    ids: &[&str],
) -> Fixture<Scripted> {
    build(source, reply, request, ids, None)
}

// The length guard turns a driver that never ends into a failure instead of a hung test
fn record(seen: &mut Vec<Json>, event: &KnownResponseEvent) {
    seen.push(event_json(event));
    assert!(seen.len() < 64, "the stream ends");
}

pub(super) fn drain(stream: &mut SyncStream) -> Vec<Json> {
    let mut seen = Vec::new();
    while let Some(event) = stream.next_event().expect("the stream does not fail") {
        record(&mut seen, &event);
    }
    seen
}

pub(super) async fn drain_async(stream: &mut SyncStream) -> Vec<Json> {
    let mut seen = Vec::new();
    while let Some(event) = stream
        .next_event_async()
        .await
        .expect("the stream does not fail")
    {
        record(&mut seen, &event);
    }
    seen
}

pub(super) fn event_json(event: &KnownResponseEvent) -> Json {
    json(&serde_json::to_string(event).expect("the event serializes"))
}

pub(super) fn kinds(events: &[Json]) -> Vec<String> {
    events
        .iter()
        .map(|event| match field(event, "type") {
            Json::Text(kind) => kind,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                panic!("the event type is text")
            }
        })
        .collect()
}
