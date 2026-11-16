#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::http::{StatusCode, header};
use axum::routing::post;
use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;
use serde_json::value::RawValue;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Receiver};
use tokio::task::JoinSet;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::cato::error::ModelStreamError;
use crate::presidio_actions::request_data::RequestData;
use crate::presidio_actions::{AnonymizerSettings, PresidioAnonymizer};
use crate::presidio_analysis::wire::{MockRedactedItem, MockRedactedText};
use crate::presidio_analysis::{AnalyzerSettings, DEFAULT_CHUNK_SIZE_BYTES, PresidioAnalyzer};
use crate::presidio_response::HookContext;
use crate::presidio_response::wire::{ChatModelResponse, Content, ToolCallItem};
use crate::presidio_streams::assembly::StreamAssembly;
use crate::presidio_streams::coalesce::{Coalesced, Coalescer};
use crate::presidio_streams::error::StreamError;
use crate::presidio_streams::hook::streaming_iterator_hook;
use crate::presidio_streams::mask::stream_apply_output_masking;
use crate::presidio_streams::unmask::stream_pii_unmasking;
use crate::presidio_streams::unmask_sse::PiiTokens;
use crate::presidio_streams::{StreamEvent, StreamRun};

pub(super) type Events = Vec<Result<StreamEvent, ModelStreamError>>;
pub(super) type Outcome = (Vec<String>, Result<(), StreamError<FakeAssembly>>);

pub(super) const START_FRAME: &str =
    "event: message_start\ndata: {\"delta\":{\"text\":\"Hello\"}}\n\n";
pub(super) const DELTA_FRAME: &str =
    "event: content_block_delta\ndata: {\"delta\":{\"text\":\"Alice\"}}\n\n";

static NO_PARAMS: BTreeMap<String, GuardrailValue> = BTreeMap::new();

#[derive(Debug, Error)]
#[error("the fake assembly failed")]
pub(super) struct FakeFailure;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Builder {
    #[default]
    Chat,
    NotAModel,
    Fails,
    Tool,
}

// Stands in for the helpers B2227 and B3669 own: it joins the delta texts of chat chunks and the text_delta texts of SSE frames
#[derive(Default)]
pub(super) struct FakeAssembly {
    pub(super) builder: Builder,
    pub(super) seen_messages: Mutex<Vec<Option<String>>>,
    pub(super) seen_frames: Mutex<Vec<Vec<Vec<u8>>>>,
}

#[derive(Deserialize)]
struct Chunk {
    choices: Vec<ChunkChoice>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    delta: ChunkDelta,
}

#[derive(Deserialize)]
struct ChunkDelta {
    content: String,
}

fn raw(json: &str) -> Box<RawValue> {
    RawValue::from_string(json.to_owned()).unwrap()
}

fn response(text: &str) -> ChatModelResponse {
    let content = serde_json::to_string(text).unwrap();
    let message =
        format!(r#"{{"choices":[{{"message":{{"role":"assistant","content":{content}}}}}]}}"#);
    serde_json::from_str(&message).unwrap()
}

fn tool_response(arguments: &str) -> ChatModelResponse {
    let encoded = serde_json::to_string(arguments).unwrap();
    let message = format!(
        r#"{{"choices":[{{"message":{{"role":"assistant","tool_calls":[{{"id":"t1","type":"function","function":{{"name":"lookup","arguments":{encoded}}}}}]}}}}]}}"#
    );
    serde_json::from_str(&message).unwrap()
}

#[derive(Deserialize)]
struct Frame {
    delta: FrameDelta,
}

#[derive(Deserialize)]
struct FrameDelta {
    text: Option<String>,
    partial_json: Option<String>,
}

impl StreamAssembly for FakeAssembly {
    type Error = FakeFailure;

    fn is_anthropic_sse_stream(&self, chunk: &[u8]) -> bool {
        str::from_utf8(chunk).is_ok_and(|text| text.contains("event: message_start"))
    }

    fn stream_chunk_builder(
        &self,
        chunks: &[Box<RawValue>],
        messages: Option<&RawValue>,
    ) -> impl Future<Output = Result<Option<ChatModelResponse>, FakeFailure>> + Send {
        self.seen_messages
            .lock()
            .unwrap()
            .push(messages.map(|value| value.get().to_owned()));
        ready(match self.builder {
            Builder::Fails => Err(FakeFailure),
            Builder::NotAModel => Ok(None),
            Builder::Chat | Builder::Tool => {
                let text: String = chunks
                    .iter()
                    .map(|chunk| serde_json::from_str::<Chunk>(chunk.get()).unwrap())
                    .flat_map(|chunk| chunk.choices)
                    .map(|choice| choice.delta.content)
                    .collect();
                Ok(Some(response(&text)))
            }
        })
    }

    fn convert_to_streaming(
        &self,
        response: &ChatModelResponse,
    ) -> Result<Box<RawValue>, FakeFailure> {
        let Ok(encoded) = serde_json::to_string(response) else {
            return Err(FakeFailure);
        };
        Ok(raw(&encoded))
    }

    fn assemble_anthropic_sse_stream(
        &self,
        chunks: &[Vec<u8>],
    ) -> impl Future<Output = Option<ChatModelResponse>> + Send {
        self.seen_frames.lock().unwrap().push(chunks.to_vec());
        let frames = String::from_utf8(chunks.concat()).unwrap();
        if self.builder == Builder::NotAModel || !frames.contains("event: message_start") {
            return ready(None);
        }
        let mut text = String::new();
        let mut arguments = String::new();
        for line in frames
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
        {
            let Ok(frame) = serde_json::from_str::<Frame>(line) else {
                continue;
            };
            if let Some(found) = frame.delta.text {
                text.push_str(&found);
            }
            if let Some(found) = frame.delta.partial_json {
                arguments.push_str(&found);
            }
        }
        ready(Some(match self.builder {
            Builder::Tool => tool_response(&arguments),
            Builder::Chat | Builder::NotAModel | Builder::Fails => response(&text),
        }))
    }

    fn model_response_text(&self, response: &ChatModelResponse) -> String {
        let content = response
            .choices
            .iter()
            .flatten()
            .filter_map(|choice| choice.message.as_ref())
            .find_map(|message| message.content.as_ref());
        match content {
            Some(Content::Text(text)) => text.clone(),
            Some(Content::Blocks(_) | Content::Other(_)) | None => String::new(),
        }
    }

    fn anthropic_sse_chunks(
        &self,
        response: &ChatModelResponse,
    ) -> Result<Vec<Vec<u8>>, FakeFailure> {
        let text = self.model_response_text(response);
        let arguments: String = response
            .choices
            .iter()
            .flatten()
            .filter_map(|choice| choice.message.as_ref())
            .filter_map(|message| message.tool_calls.as_ref())
            .flatten()
            .filter_map(|call| {
                let ToolCallItem::Object(call) = call else {
                    return None;
                };
                call.function.as_ref()
            })
            .filter_map(|function| function.arguments.clone())
            .collect();
        let rebuilt = if arguments.is_empty() {
            format!("rebuilt:{text}")
        } else {
            format!("rebuilt:{text};args:{arguments}")
        };
        Ok(vec![rebuilt.into_bytes()])
    }
}

pub(super) fn chat_chunk(text: &str) -> Box<RawValue> {
    let content = serde_json::to_string(text).unwrap();
    raw(&format!(
        r#"{{"object":"chat.completion.chunk","choices":[{{"delta":{{"content":{content}}}}}]}}"#
    ))
}

pub(super) fn chat(text: &str) -> StreamEvent {
    StreamEvent::Chat(chat_chunk(text))
}

pub(super) fn chat_with_usage(text: &str) -> StreamEvent {
    let content = serde_json::to_string(text).unwrap();
    StreamEvent::Chat(raw(&format!(
        r#"{{"choices":[{{"delta":{{"content":{content}}}}}],"usage":{{"total_tokens":7}}}}"#
    )))
}

pub(super) fn bytes(frame: &str) -> StreamEvent {
    StreamEvent::Bytes(frame.as_bytes().to_vec())
}

pub(super) fn other(json: &str) -> StreamEvent {
    StreamEvent::Other(raw(json))
}

pub(super) fn ok(events: Vec<StreamEvent>) -> Events {
    events.into_iter().map(Ok).collect()
}

pub(super) fn receiver(events: Events) -> Receiver<Result<StreamEvent, ModelStreamError>> {
    let (sender, receiver) = mpsc::channel(events.len().max(1));
    for event in events {
        sender.try_send(event).unwrap();
    }
    receiver
}

// What the coalescer hands out for the events, one line each
pub(super) async fn coalesce(events: Events) -> Vec<String> {
    let mut input = receiver(events);
    let mut stream = Coalescer {
        input: &mut input,
        pending: Vec::new(),
        searching: true,
        ready: VecDeque::new(),
    };
    let mut lines = Vec::new();
    while let Some(item) = stream.coalesce_first_sse_frame().await {
        lines.push(match item {
            Ok(Coalesced::Preface(frames)) => {
                format!("preface {}", str::from_utf8(&frames).unwrap())
            }
            Ok(Coalesced::Event(event)) => describe(&event),
            Err(_failure) => "error".to_owned(),
        });
    }
    lines
}

pub(super) fn upstream_failure() -> ModelStreamError {
    ModelStreamError::Provider {
        message: Box::from("boom"),
    }
}

// One line per event, so a test compares a whole output with one assert
pub(super) fn describe(event: &StreamEvent) -> String {
    match event {
        StreamEvent::Chat(chunk) => format!("chat {}", chunk.get()),
        StreamEvent::Bytes(frame) => format!("bytes {}", str::from_utf8(frame).unwrap()),
        StreamEvent::Other(other_event) => format!("other {}", other_event.get()),
    }
}

pub(super) fn request_data(json: &str) -> RequestData {
    serde_json::from_str(json).unwrap()
}

pub(super) fn tokens() -> PiiTokens {
    PiiTokens::from([("<PERSON_1>".to_owned(), "Alice".to_owned())])
}

pub(super) fn tokens_request() -> RequestData {
    request_data(
        r#"{"metadata":{"pii_tokens":{"<PERSON_1>":"Alice"}},"messages":[{"role":"user","content":"hi"}]}"#,
    )
}

pub(super) struct Harness {
    pub(super) analyzer: PresidioAnalyzer,
    pub(super) anonymizer: PresidioAnonymizer,
    pub(super) assembly: FakeAssembly,
    pub(super) entities: BTreeMap<String, PiiAction>,
    pub(super) reader_gone: bool,
    pub(super) global_output_parse_pii: bool,
}

fn fixed_clock() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .unwrap()
}

impl Harness {
    // Every text is masked to "MASKED" without a Presidio call
    pub(super) fn new() -> Self {
        Self::with_base("http://presidio.invalid", Some("MASKED"))
    }

    pub(super) fn with_base(base: &str, masked: Option<&str>) -> Self {
        let mock = masked.map(|text| MockRedactedText {
            text: text.to_owned(),
            items: vec![MockRedactedItem {
                entity_type: Some("PERSON".to_owned()),
            }],
        });
        Self {
            analyzer: PresidioAnalyzer {
                settings: AnalyzerSettings {
                    guardrail_name: "presidio-global".to_owned(),
                    analyzer_api_base: format!("{base}/"),
                    language: "en".to_owned(),
                    ad_hoc_recognizers: None,
                    pii_entities: Vec::new(),
                    output_parse_pii: false,
                    apply_to_output: false,
                    score_thresholds: BTreeMap::new(),
                    entities_deny_list: Vec::new(),
                    chunk_size_bytes: DEFAULT_CHUNK_SIZE_BYTES,
                    timeout: Duration::from_secs(5),
                    mock_redacted_text: mock,
                },
                client: reqwest::Client::default(),
                chunk_semaphores: Mutex::default(),
            },
            anonymizer: PresidioAnonymizer {
                settings: AnonymizerSettings {
                    anonymizer_api_base: format!("{base}/"),
                    timeout: Duration::from_secs(5),
                },
                client: reqwest::Client::default(),
                clock: fixed_clock,
            },
            assembly: FakeAssembly::default(),
            entities: BTreeMap::new(),
            reader_gone: false,
            global_output_parse_pii: false,
        }
    }

    pub(super) fn context(&self) -> HookContext<'_> {
        HookContext {
            analyzer: &self.analyzer,
            anonymizer: &self.anonymizer,
            pii_entities_config: &self.entities,
            dynamic_params: &NO_PARAMS,
            global_output_parse_pii: self.global_output_parse_pii,
        }
    }

    // Runs one hook entry over the events and reports what it sent and how it ended
    pub(super) async fn run(
        &self,
        kind: Kind,
        request: &mut RequestData,
        events: Events,
    ) -> Outcome {
        let (out, mut sent) = mpsc::channel(64);
        if self.reader_gone {
            sent.close();
        }
        let mut run = StreamRun {
            context: self.context(),
            request_data: request,
            assembly: &self.assembly,
            out: &out,
        };
        let mut input = receiver(events);
        let result = match kind {
            Kind::Mask => stream_apply_output_masking(&mut run, &mut input).await,
            Kind::Unmask => stream_pii_unmasking(&mut run, &mut input).await,
            Kind::Hook => streaming_iterator_hook(&mut run, &mut input).await,
        };
        drop(out);
        let mut described = Vec::new();
        while let Some(event) = sent.recv().await {
            described.push(describe(&event));
        }
        (described, result)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Mask,
    Unmask,
    Hook,
}

// An analyzer that finds a PERSON in every text
pub(super) async fn spawn_person_analyzer() -> (String, JoinSet<io::Result<()>>) {
    let reply = || async {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            r#"[{"entity_type":"PERSON","start":0,"end":5,"score":0.9}]"#,
        )
    };
    let router = Router::new().route("/analyze", post(reply));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut captured = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        captured.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture_logs() -> (Arc<Mutex<Vec<u8>>>, DefaultGuard) {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let sink = Sink(Arc::clone(&captured));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_writer(move || sink.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn logged_text(captured: &Mutex<Vec<u8>>) -> String {
    String::from_utf8(captured.lock().unwrap().clone()).unwrap()
}
