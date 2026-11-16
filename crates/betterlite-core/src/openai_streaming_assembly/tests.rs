#![cfg(test)]

use std::fmt::Debug;
use std::future::Future;
use std::mem;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_chunk_shaping::records::JsonValue;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::{
    CostError, CostInput, StreamCostCalculator,
};
use betterlite_streaming::streamed_response_assembly::response::{ChatResponse, Response};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, MessageContent, Part};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;
use uuid::Uuid;

use super::{
    RelayAssemblyRequest, RelayTokenCounter, RelayedCounter, complete_streaming_response,
    count_relayed_prompt_tokens, is_remote_high_detail_image,
};

const MODEL: &str = "relay-test-model";
const CHUNK_ID: &str = "chatcmpl-1";
const CREATED: i64 = 1_700_000_000;
const REMOTE_BOUND: i64 = 1445;

fn messages(raw: Value) -> Vec<Message> {
    serde_json::from_value(raw).unwrap()
}

fn summarize(messages: &[Message]) -> String {
    messages
        .iter()
        .map(describe_message)
        .collect::<Vec<String>>()
        .join("|")
}

fn describe_message(message: &Message) -> String {
    let content = match &message.content {
        Some(MessageContent::Text(text)) => text.clone(),
        Some(MessageContent::Parts(parts)) => parts
            .iter()
            .map(describe_part)
            .collect::<Vec<String>>()
            .join(","),
        None => String::from("none"),
    };
    format!("{}:{content}", message.role)
}

fn describe_part(part: &Part) -> String {
    if is_remote_high_detail_image(part) {
        return String::from("remote");
    }
    match part {
        Part::Text(text) => format!("text({text})"),
        Part::Block(block) => block.kind.clone(),
    }
}

#[derive(Default)]
struct FixtureCounter {
    messages: i64,
    text: i64,
    response_text: i64,
    fail_messages: bool,
    calls: Mutex<Vec<String>>,
}

impl FixtureCounter {
    fn new(messages: i64, response_text: i64) -> Self {
        Self {
            messages,
            text: 0,
            response_text,
            fail_messages: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(messages: i64) -> Self {
        Self {
            messages,
            text: 0,
            response_text: 0,
            fail_messages: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RelayTokenCounter for FixtureCounter {
    fn count_messages(&self, _model: &str, messages: &[Message]) -> Result<i64, CountError> {
        self.record(format!("messages:{}", summarize(messages)));
        if self.fail_messages {
            return Err(CountError::MissingInput);
        }
        Ok(self.messages)
    }

    fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.record(format!("text:{text}"));
        Ok(self.text)
    }

    fn count_response_text(&self, _model: &str, text: &str) -> Result<i64, CountError> {
        self.record(format!("response:{text}"));
        Ok(self.response_text)
    }
}

struct NoCost;

impl StreamCostCalculator for NoCost {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(None)
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Err(CostError {
            message: String::from("the relayed model is not priced"),
        })
    }
}

struct Priced;

impl StreamCostCalculator for Priced {
    fn completion_cost(
        &self,
        _input: &CostInput<'_>,
        _provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError> {
        Ok(Some(Decimal::new(25, 7)))
    }

    fn prompt_completion_cost(
        &self,
        _model: &str,
        _usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError> {
        Ok((Decimal::ZERO, Decimal::ZERO))
    }
}

#[derive(Default)]
struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        true
    }

    fn error(&self, _payload: &Payload) -> bool {
        true
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

fn hooks() -> VerboseHooks<SilentLogger, SilentConsole> {
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    }
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut lines) => mem::take(&mut lines),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct EventMessage(String);

impl Visit for EventMessage {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::ERROR {
            return;
        }
        let mut message = EventMessage::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{level} {}", message.0));
    }
}

async fn capture<F>(call: F) -> (Option<Response>, Vec<String>)
where
    F: Future<Output = Option<Response>>,
{
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let guard = set_default(subscriber);
    let response = call.await;
    drop(guard);
    (response, logs.take())
}

fn chat(response: &Response) -> &ChatResponse {
    match response {
        Response::Chat(chat) => chat,
        Response::Text(text) => panic!("the relay assembled a text response: {text:?}"),
    }
}

fn chunk(choices: Value) -> Value {
    json!({
        "id": CHUNK_ID,
        "object": "chat.completion.chunk",
        "created": CREATED,
        "model": MODEL,
        "choices": choices,
    })
}

fn delta_chunk(delta: Value, finish_reason: Option<&str>) -> String {
    let mut choice = json!({ "index": 0, "delta": delta });
    if let Some(reason) = finish_reason {
        choice["finish_reason"] = json!(reason);
    }
    chunk(json!([choice])).to_string()
}

fn usage_chunk(prompt_tokens: i64, completion_tokens: i64) -> String {
    let mut body = chunk(json!([{ "index": 0, "delta": {}, "finish_reason": "stop" }]));
    body["usage"] = json!({
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": prompt_tokens + completion_tokens,
    });
    body.to_string()
}

fn content_chunks() -> Vec<String> {
    vec![
        delta_chunk(json!({ "role": "assistant", "content": "Hello" }), None),
        delta_chunk(json!({ "content": " world" }), Some("stop")),
    ]
}

fn chunk_id() -> Uuid {
    Uuid::nil()
}

fn chunk_created() -> i64 {
    0
}

fn request<'request>(
    chunks: &'request [String],
    messages: Option<&'request [Message]>,
    counter: &'request dyn RelayTokenCounter,
    costs: &'request dyn StreamCostCalculator,
) -> RelayAssemblyRequest<'request> {
    RelayAssemblyRequest {
        chunks,
        model: MODEL,
        messages,
        counter,
        costs,
        identify: chunk_id,
        clock: chunk_created,
    }
}

#[test]
fn count_relayed_prompt_tokens_is_zero_without_messages() {
    let counter = FixtureCounter::new(9, 0);
    let counted = count_relayed_prompt_tokens(&counter, MODEL, None);
    assert_eq!(counted.ok(), Some(0));
    assert!(counter.calls().is_empty());
}

#[test]
fn count_relayed_prompt_tokens_leaves_a_text_content_message_alone() {
    let counter = FixtureCounter::new(9, 0);
    let raw = json!([{ "role": "user", "content": "hello world" }]);
    let typed = messages(raw);
    let counted = count_relayed_prompt_tokens(&counter, MODEL, Some(&typed));
    assert_eq!(counted.ok(), Some(9));
    assert_eq!(
        counter.calls(),
        vec![String::from("messages:user:hello world")]
    );
}

#[test]
fn count_relayed_prompt_tokens_drops_a_remote_image_and_adds_its_bound() {
    let counter = FixtureCounter::new(8, 0);
    let raw = json!([{
        "role": "user",
        "content": [
            { "type": "text", "text": "describe" },
            { "type": "image_url", "image_url": { "url": "https://example.com/a.png", "detail": "high" } },
        ],
    }]);
    let typed = messages(raw);
    let counted = count_relayed_prompt_tokens(&counter, MODEL, Some(&typed));
    assert_eq!(counted.ok(), Some(8 + REMOTE_BOUND));
    assert_eq!(counter.calls(), vec![String::from("messages:user:text")]);
}

#[test]
fn count_relayed_prompt_tokens_adds_the_bound_for_each_remote_image() {
    let counter = FixtureCounter::new(4, 0);
    let raw = json!([
        {
            "role": "system",
            "content": [
                { "type": "image_url", "image_url": { "url": "http://example.com/a.png", "detail": "high" } },
            ],
        },
        {
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": "HTTPS://EXAMPLE.COM/b.png", "detail": "high" } },
                { "type": "image_url", "image_url": { "url": "https://example.com/c.png", "detail": "low" } },
                { "type": "image_url", "image_url": { "url": "https://example.com/d.png" } },
            ],
        },
    ]);
    let typed = messages(raw);
    let counted = count_relayed_prompt_tokens(&counter, MODEL, Some(&typed));
    assert_eq!(counted.ok(), Some(4 + 2 * REMOTE_BOUND));
    assert_eq!(
        counter.calls(),
        vec![String::from("messages:system:|user:image_url,image_url")]
    );
}

#[rstest]
#[case::low_detail(
    r#"{"type":"image_url","image_url":{"url":"https://example.com/a.png","detail":"low"}}"#
)]
#[case::missing_detail(r#"{"type":"image_url","image_url":{"url":"https://example.com/a.png"}}"#)]
#[case::string_image_url(r#"{"type":"image_url","image_url":"https://example.com/a.png"}"#)]
#[case::ftp_url(
    r#"{"type":"image_url","image_url":{"url":"ftp://example.com/a.png","detail":"high"}}"#
)]
#[case::other_part_type(
    r#"{"type":"image","image_url":{"url":"https://example.com/a.png","detail":"high"}}"#
)]
#[case::data_url(
    r#"{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA","detail":"high"}}"#
)]
#[case::bare_string_part(r#""https://example.com/a.png""#)]
fn count_relayed_prompt_tokens_keeps_a_part_that_is_not_a_remote_high_detail_image(
    #[case] part: &str,
) {
    let counter = FixtureCounter::new(5, 0);
    let part: Value = serde_json::from_str(part).unwrap();
    let raw = json!([{ "role": "user", "content": [part] }]);
    let typed = messages(raw);
    let counted = count_relayed_prompt_tokens(&counter, MODEL, Some(&typed));
    assert_eq!(counted.ok(), Some(5));
    assert_eq!(counter.calls().len(), 1);
    assert!(!counter.calls()[0].contains("remote"));
}

#[test]
fn count_relayed_prompt_tokens_reports_a_counter_failure() {
    let counter = FixtureCounter::failing(9);
    let raw = json!([{ "role": "user", "content": "hello" }]);
    let typed = messages(raw);
    let counted = count_relayed_prompt_tokens(&counter, MODEL, Some(&typed));
    assert_eq!(
        counted.err().map(|error| error.to_string()),
        Some(String::from("either text or messages must be provided"))
    );
}

#[test]
fn count_relayed_prompt_tokens_reports_an_overflowing_sum() {
    let counter = FixtureCounter::new(i64::MAX, 0);
    let raw = json!([{
        "role": "user",
        "content": [
            { "type": "image_url", "image_url": { "url": "https://example.com/a.png", "detail": "high" } },
        ],
    }]);
    let typed = messages(raw);
    let counted = count_relayed_prompt_tokens(&counter, MODEL, Some(&typed));
    assert_eq!(
        counted.err().map(|error| error.to_string()),
        Some(String::from(
            "the token count does not fit in a machine word"
        ))
    );
}

#[tokio::test]
async fn assembly_builds_one_response_from_prefixed_and_bare_chunks() {
    let content = content_chunks();
    let chunks = vec![format!("data: {}", content[0]), content[1].clone()];
    let counter = FixtureCounter::new(0, 2);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.id, CHUNK_ID);
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.created, CREATED);
    assert_eq!(response.model, MODEL);
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("Hello world")
    );
    assert_eq!(response.choices[0].finish_reason, "stop");
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 2);
    assert_eq!(counter.calls(), vec![String::from("response:Hello world")]);
}

#[rstest]
#[case::not_json("not json")]
#[case::done_marker("data: [DONE]")]
#[case::empty_string("")]
#[case::empty_object("{}")]
#[case::json_array("[1, 2]")]
#[case::json_string("\"text\"")]
#[case::json_null("null")]
#[case::prefix_only("data: ")]
#[case::prefix_with_broken_json("data:{broken")]
#[tokio::test]
async fn assembly_skips_a_malformed_chunk_and_keeps_the_valid_one(#[case] malformed: &str) {
    let valid = delta_chunk(json!({ "content": "Hi" }), Some("stop"));
    let chunks = vec![String::from(malformed), valid];
    let counter = FixtureCounter::new(0, 1);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let (response, logs) = capture(complete_streaming_response(&assembly, &hooks())).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.choices[0].message.content.as_deref(), Some("Hi"));
    assert_eq!(response.usage.completion_tokens, 1);
    assert!(logs.is_empty(), "a malformed chunk logs nothing: {logs:?}");
}

#[tokio::test]
async fn assembly_returns_none_and_warns_when_no_chunk_parses() {
    let chunks = vec![
        String::from("not json"),
        String::from("data: [DONE]"),
        String::from("{}"),
    ];
    let counter = FixtureCounter::new(0, 0);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let (response, logs) = capture(complete_streaming_response(&assembly, &hooks())).await;
    assert!(response.is_none());
    assert_eq!(
        logs,
        vec![String::from(
            "WARN No valid chunks found in streaming response"
        )]
    );
    assert!(counter.calls().is_empty());
}

#[tokio::test]
async fn assembly_returns_none_and_warns_for_an_empty_chunk_list() {
    let chunks: Vec<String> = Vec::new();
    let counter = FixtureCounter::new(0, 0);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let (response, logs) = capture(complete_streaming_response(&assembly, &hooks())).await;
    assert!(response.is_none());
    assert_eq!(logs.len(), 1);
    assert!(logs[0].contains("No valid chunks found in streaming response"));
}

#[tokio::test]
async fn assembly_skips_a_provider_error_chunk() {
    let chunks = vec![json!({ "error": { "message": "boom", "code": 500 } }).to_string()];
    let counter = FixtureCounter::new(0, 0);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let (response, logs) = capture(complete_streaming_response(&assembly, &hooks())).await;
    assert!(response.is_none());
    assert!(
        logs.iter()
            .any(|line| line.contains("No valid chunks found"))
    );
}

#[tokio::test]
async fn assembly_logs_an_error_and_returns_none_when_the_builder_fails() {
    let chunks = vec![
        json!({
            "choices": [{ "index": 0, "delta": { "content": "Hi" }, "finish_reason": "stop" }],
        })
        .to_string(),
    ];
    let counter = FixtureCounter::new(0, 0);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let (response, logs) = capture(complete_streaming_response(&assembly, &hooks())).await;
    assert!(response.is_none());
    assert_eq!(logs.len(), 1);
    assert!(
        logs[0].starts_with("ERROR Error building complete streaming response: "),
        "unexpected log: {:?}",
        logs[0]
    );
}

#[tokio::test]
async fn assembly_counts_the_prompt_through_the_relayed_counter() {
    let chunks = content_chunks();
    let raw = json!([{
        "role": "user",
        "content": [
            { "type": "text", "text": "describe" },
            { "type": "image_url", "image_url": { "url": "https://example.com/a.png", "detail": "high" } },
        ],
    }]);
    let typed = messages(raw);
    let counter = FixtureCounter::new(8, 2);
    let assembly = request(&chunks, Some(&typed), &counter, &NoCost);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.usage.prompt_tokens, 8 + REMOTE_BOUND);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 10 + REMOTE_BOUND);
    assert_eq!(
        counter.calls(),
        vec![
            String::from("messages:user:text"),
            String::from("response:Hello world"),
        ]
    );
}

#[tokio::test]
async fn assembly_skips_the_prompt_count_when_a_chunk_reports_usage() {
    let chunks = vec![usage_chunk(7, 2)];
    let raw = json!([{ "role": "user", "content": "hello world" }]);
    let typed = messages(raw);
    let counter = FixtureCounter::new(9, 2);
    let assembly = request(&chunks, Some(&typed), &counter, &NoCost);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.usage.prompt_tokens, 7);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 9);
    assert!(counter.calls().is_empty());
}

#[tokio::test]
async fn assembly_treats_a_null_usage_chunk_as_absent() {
    let mut body =
        chunk(json!([{ "index": 0, "delta": { "content": "Hi" }, "finish_reason": "stop" }]));
    body["usage"] = Value::Null;
    let chunks = vec![body.to_string()];
    let raw = json!([{ "role": "user", "content": "hello world" }]);
    let typed = messages(raw);
    let counter = FixtureCounter::new(9, 1);
    let assembly = request(&chunks, Some(&typed), &counter, &NoCost);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.usage.prompt_tokens, 9);
    assert_eq!(response.usage.completion_tokens, 1);
}

#[tokio::test]
async fn assembly_keeps_the_system_fingerprint_from_the_chunk() {
    let mut body =
        chunk(json!([{ "index": 0, "delta": { "content": "Hi" }, "finish_reason": "stop" }]));
    body["system_fingerprint"] = json!("fp_abc");
    let chunks = vec![body.to_string()];
    let counter = FixtureCounter::new(0, 1);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp_abc"));
}

#[tokio::test]
async fn assembly_fills_a_missing_chunk_id_and_created_from_the_caller() {
    let chunks = vec![
        json!({
            "model": MODEL,
            "choices": [{ "index": 0, "delta": { "content": "Hi" }, "finish_reason": "stop" }],
        })
        .to_string(),
    ];
    let counter = FixtureCounter::new(0, 1);
    let assembly = request(&chunks, None, &counter, &NoCost);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.id, format!("chatcmpl-{}", Uuid::nil()));
    assert_eq!(response.created, 0);
}

#[tokio::test]
async fn assembly_stamps_the_cost_of_the_given_calculator() {
    let chunks = content_chunks();
    let counter = FixtureCounter::new(0, 2);
    let assembly = request(&chunks, None, &counter, &Priced);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    let cost = Number::from_str("0.0000025").unwrap();
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&JsonValue::Number(cost))
    );
}

#[tokio::test]
async fn assembly_counts_reasoning_tokens_through_the_relayed_counter() {
    let chunks = vec![delta_chunk(
        json!({ "reasoning_content": "think", "content": "Hi" }),
        Some("stop"),
    )];
    let counter = FixtureCounter {
        messages: 0,
        text: 3,
        response_text: 2,
        fail_messages: false,
        calls: Mutex::new(Vec::new()),
    };
    let assembly = request(&chunks, None, &counter, &NoCost);
    let response = complete_streaming_response(&assembly, &hooks()).await;
    let response = chat(response.as_ref().unwrap());
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 5);
    assert_eq!(
        counter.calls(),
        vec![String::from("text:think"), String::from("response:Hi")]
    );
}

#[tokio::test]
async fn relayed_counter_counts_messages_through_the_prompt_estimate() {
    let raw = json!([{
        "role": "user",
        "content": [
            { "type": "text", "text": "describe" },
            { "type": "image_url", "image_url": { "url": "https://example.com/a.png", "detail": "high" } },
        ],
    }]);
    let typed = messages(raw);
    let counter = FixtureCounter::new(8, 0);
    let relayed = RelayedCounter { counter: &counter };
    let counted = UsageTokenCounter::count_messages(&relayed, MODEL, Some(&typed)).await;
    assert_eq!(counted.ok(), Some(8 + REMOTE_BOUND));
    assert_eq!(counter.calls(), vec![String::from("messages:user:text")]);
    let failing = FixtureCounter::failing(9);
    let relayed = RelayedCounter { counter: &failing };
    let counted = UsageTokenCounter::count_messages(&relayed, MODEL, Some(&typed)).await;
    assert_eq!(
        counted.err().map(|error| error.to_string()),
        Some(String::from("either text or messages must be provided"))
    );
}
