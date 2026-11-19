#![cfg(test)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::system_placement::text_parts::CONVERTED_SYSTEM_NOTE;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::json;
use uuid::Uuid;

use crate::api::messages_chat_output::records::ResponseRecord;
use crate::api::messages_chat_stream::sse::SyncFrames;
use crate::api::messages_chat_stream::view::AnthropicSseStream;
use crate::api::messages_chat_tools_schema::tool_choice::ToolChoiceError;
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, PolyfillResult, UsageIteration, UsageKind,
};
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::messages_stream_split::{AsyncChunkSource, ChunkSource};

use super::adapter::AnthropicAdapter;
use super::compose::translate_anthropic_to_openai;
use super::error::TransformError;
use super::settings::AdapterSettings;
use super::stream::{StreamOptions, StreamOutput};

const MODEL_REQUIRED: &str = "Bad Request: model is required for Anthropic Messages Request";
const MESSAGES_REQUIRED: &str = "Bad Request: messages is required for Anthropic Messages Request";

type AsyncView = AnthropicSseStream<Scripted, FixedIds>;
type SyncIterator = SyncFrames<Scripted, FixedIds>;

// LiteLLM reads the capability from its pinned model map, so a test says which models it flags.
struct Capabilities {
    models: Vec<String>,
    providers: Vec<String>,
    probes: Vec<String>,
    provider_probes: Vec<Option<String>>,
}

impl Capabilities {
    fn new(models: &[&str]) -> Self {
        Self {
            models: models.iter().map(|model| String::from(*model)).collect(),
            providers: Vec::new(),
            probes: Vec::new(),
            provider_probes: Vec::new(),
        }
    }
}

impl CapabilityPredicates for Capabilities {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.probes.push(request.model.to_owned());
        self.provider_probes
            .push(request.custom_llm_provider.map(str::to_owned));
        let flagged = key == NamedCapability::MidConversationSystem.key()
            && (self.models.iter().any(|model| model == request.model)
                || self
                    .providers
                    .iter()
                    .any(|provider| request.custom_llm_provider == Some(provider.as_str())));
        ready(flagged)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = DecisionJson> + Send {
        ready(DecisionJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

// The provider's declared parameters are unknown here, so every option step reads them as absent.
struct Declared;

impl SupportedParameterSource for Declared {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("a valid object fixture")
}

fn adapter() -> AnthropicAdapter {
    AnthropicAdapter::new()
}

async fn wrap(
    kwargs: &mut Object,
    capabilities: &mut Capabilities,
) -> Result<(Object, IndexMap<String, String>), TransformError> {
    let declared = Declared;
    adapter()
        .translate_completion_input_params_with_tool_mapping(
            kwargs,
            &AdapterSettings::new(&declared),
            capabilities,
        )
        .await
}

async fn compose(
    request: &mut Object,
    preserve_midturn_system: bool,
    capabilities: &mut Capabilities,
) -> Result<(Object, IndexMap<String, String>), TransformError> {
    let declared = Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.preserve_midturn_system = preserve_midturn_system;
    translate_anthropic_to_openai(request, &settings, capabilities).await
}

#[tokio::test]
async fn both_required_keys_leave_kwargs_before_either_is_checked() {
    let mut missing_model = object(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    let error = wrap(&mut missing_model, &mut Capabilities::new(&[]))
        .await
        .expect_err("a request without a model cannot compose");
    assert!(matches!(error, TransformError::MissingKey { key: "model" }));
    assert_eq!(error.to_string(), "'model'");
    assert_eq!(
        missing_model,
        object(r#"{"messages": [{"role": "user", "content": "hi"}]}"#),
        "the messages stay in kwargs when model is missing"
    );

    let mut missing_messages = object(r#"{"model": "gpt-4o"}"#);
    let error = wrap(&mut missing_messages, &mut Capabilities::new(&[]))
        .await
        .expect_err("a request without messages cannot compose");
    assert!(matches!(
        error,
        TransformError::MissingKey { key: "messages" }
    ));
    assert_eq!(error.to_string(), "'messages'");
    assert!(
        missing_messages.is_empty(),
        "model was popped before the missing messages key raised"
    );
}

#[tokio::test]
async fn empty_required_keys_raise_litellms_value_errors() {
    let mut empty_model =
        object(r#"{"model": "", "messages": [{"role": "user", "content": "hi"}]}"#);
    let error = wrap(&mut empty_model, &mut Capabilities::new(&[]))
        .await
        .expect_err("an empty model is refused");
    assert!(matches!(error, TransformError::ModelRequired));
    assert_eq!(error.to_string(), MODEL_REQUIRED);

    let mut empty_messages = object(r#"{"model": "gpt-4o", "messages": []}"#);
    let error = wrap(&mut empty_messages, &mut Capabilities::new(&[]))
        .await
        .expect_err("empty messages are refused");
    assert!(matches!(error, TransformError::MessagesRequired));
    assert_eq!(error.to_string(), MESSAGES_REQUIRED);

    let declared = Declared;
    let mut convenience =
        object(r#"{"model": "", "messages": [{"role": "user", "content": "hi"}]}"#);
    let error = adapter()
        .translate_completion_input_params(
            &mut convenience,
            &AdapterSettings::new(&declared),
            &mut Capabilities::new(&[]),
        )
        .await
        .expect_err("the convenience wrapper validates the same way");
    assert!(matches!(error, TransformError::ModelRequired));
}

#[tokio::test]
async fn non_text_required_keys_are_typed_errors() {
    let mut numeric_model =
        object(r#"{"model": 7, "messages": [{"role": "user", "content": "hi"}]}"#);
    let error = wrap(&mut numeric_model, &mut Capabilities::new(&[]))
        .await
        .expect_err("a numeric model is refused");
    assert!(matches!(error, TransformError::ModelNotText));

    let mut text_messages = object(r#"{"model": "gpt-4o", "messages": "hi"}"#);
    let error = wrap(&mut text_messages, &mut Capabilities::new(&[]))
        .await
        .expect_err("a string messages value is refused");
    assert!(matches!(error, TransformError::MessagesNotList));
}

#[tokio::test]
async fn composed_chat_request_carries_every_translated_option() {
    let mut kwargs = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "system": "guide",
            "max_tokens": 12,
            "metadata": {"user_id": "alice"},
            "litellm_metadata": {"tag": "t"},
            "stop_sequences": ["END"],
            "tool_choice": {"type": "any"}
        }"#,
    );
    let (request, mapping) = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert!(mapping.is_empty(), "tools sent nothing to map");
    assert_eq!(
        request,
        object(
            r#"{
                "model": "gpt-4o",
                "messages": [
                    {"role": "system", "content": "guide"},
                    {"role": "user", "content": "hello"}
                ],
                "max_tokens": 12,
                "user": "alice",
                "metadata": {"tag": "t"},
                "tool_choice": "required",
                "stop": ["END"]
            }"#
        )
    );
    assert_eq!(
        kwargs,
        object(
            r#"{
                "system": "guide",
                "max_tokens": 12,
                "metadata": {"user_id": "alice"},
                "litellm_metadata": {"tag": "t"},
                "stop_sequences": ["END"],
                "tool_choice": {"type": "any"}
            }"#
        ),
        "the wrapper pops only the two required keys from the caller's kwargs"
    );
}

#[tokio::test]
async fn composition_removes_litellm_metadata_from_the_request_it_was_given() {
    let mut request = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "litellm_metadata": {"tag": "t"}
        }"#,
    );
    let (kwargs, _mapping) = compose(&mut request, false, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert!(
        request.get("litellm_metadata").is_none(),
        "the metadata moved out of the request record"
    );
    assert_eq!(kwargs.get("metadata"), Some(&json(r#"{"tag": "t"}"#)));
}

#[tokio::test]
async fn an_explicit_stop_overwrites_the_translated_stop_sequences() {
    let mut kwargs = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "stop_sequences": ["END"],
            "stop": ["LAST"]
        }"#,
    );
    let (request, _mapping) = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert_eq!(request.get("stop"), Some(&json(r#"["LAST"]"#)));
    assert!(
        request.get("stop_sequences").is_none(),
        "the translated key is not copied through a second time"
    );
}

#[tokio::test]
async fn long_tool_names_roundtrip_through_the_returned_mapping() {
    let long = "a".repeat(65);
    let short = format!("{}_635361c4", "a".repeat(55));
    let tools = json(&format!(
        r#"[{{"name": "{long}", "input_schema": {{"type": "object"}}}}]"#
    ));
    let choice = json(&format!(r#"{{"type": "tool", "name": "{long}"}}"#));
    let messages = json(r#"[{"role": "user", "content": "hi"}]"#);
    let mut kwargs = Object::new();
    kwargs.insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
    kwargs.insert("messages".to_owned(), messages.clone());
    kwargs.insert("tools".to_owned(), tools.clone());
    kwargs.insert("tool_choice".to_owned(), choice.clone());
    let (request, mapping) = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert_eq!(mapping.len(), 1);
    assert_eq!(mapping.get(&short), Some(&long));
    assert_eq!(
        request.get("tools"),
        Some(&json(&format!(
            r#"[{{"type": "function", "function": {{"name": "{short}"}}}}]"#
        )))
    );
    assert_eq!(
        request.get("tool_choice"),
        Some(&json(&format!(
            r#"{{"type": "function", "function": {{"name": "{short}"}}}}"#
        )))
    );

    let record: ResponseRecord = serde_json::from_value(json!({
        "id": "chat_a",
        "model": "gpt-4o",
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "function": {"name": short, "arguments": "{\"x\":1}"}
                }]
            }
        }]
    }))
    .expect("the response record decodes");
    let message = adapter()
        .translate_completion_output_params(&record, Some(&mapping), None)
        .expect("the response translates");
    let value = serde_json::to_value(&message).expect("the message encodes");
    assert_eq!(value["content"][0]["type"], "tool_use");
    assert_eq!(value["content"][0]["name"], long);
    assert_eq!(value["stop_reason"], "tool_use");

    let mut again = Object::new();
    again.insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
    again.insert("messages".to_owned(), messages);
    again.insert("tools".to_owned(), tools);
    again.insert("tool_choice".to_owned(), choice);
    let declared = Declared;
    let convenience = adapter()
        .translate_completion_input_params(
            &mut again,
            &AdapterSettings::new(&declared),
            &mut Capabilities::new(&[]),
        )
        .await
        .expect("the convenience wrapper composes");
    assert_eq!(
        convenience, request,
        "the convenience wrapper returns the same request without the map"
    );
}

#[tokio::test]
async fn preserve_midturn_system_reaches_the_conversation_translation() {
    let request_json = r#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "hi"},
            {"role": "system", "content": "late"}
        ]
    }"#;
    let mut preserved = object(request_json);
    let mut capabilities = Capabilities::new(&[]);
    let (kept, _mapping) = compose(&mut preserved, true, &mut capabilities)
        .await
        .expect("the request composes");
    assert_eq!(
        kept.get("messages"),
        Some(&json(
            r#"[{"role": "user", "content": "hi"}, {"role": "system", "content": "late"}]"#
        ))
    );
    assert!(
        capabilities.probes.is_empty(),
        "the preserve flag decides before any capability lookup"
    );

    let mut converted = object(request_json);
    let mut capabilities = Capabilities::new(&[]);
    let (folded, _mapping) = compose(&mut converted, false, &mut capabilities)
        .await
        .expect("the request composes");
    assert_eq!(
        folded.get("messages"),
        Some(&json(&format!(
            r#"[{{"role": "user", "content": "hi"}},
                {{"role": "user", "content": [
                    {{"type": "text", "text": "{CONVERTED_SYSTEM_NOTE}"}},
                    {{"type": "text", "text": "late"}}
                ]}}]"#
        ))),
        "an unsupported trailing system turn becomes a user turn"
    );
    assert_eq!(
        capabilities.probes,
        ["gpt-4o".to_owned()],
        "the model reaches the capability lookup"
    );
}

#[tokio::test]
async fn the_convenience_wrapper_composes_without_a_provider() {
    let request_json = r#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "hi"},
            {"role": "system", "content": "late"}
        ]
    }"#;
    let declared = Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.custom_llm_provider = Some("openai");
    let mut routed_capabilities = Capabilities::new(&[]);
    routed_capabilities.providers.push("openai".to_owned());
    let mut routed = object(request_json);
    let (kept, _mapping) = adapter()
        .translate_completion_input_params_with_tool_mapping(
            &mut routed,
            &settings,
            &mut routed_capabilities,
        )
        .await
        .expect("the routed request composes");
    assert_eq!(
        kept.get("messages"),
        Some(&json(
            r#"[{"role": "user", "content": "hi"}, {"role": "system", "content": "late"}]"#
        )),
        "the named provider decides the trailing system turn"
    );
    assert_eq!(
        routed_capabilities.provider_probes,
        [Some("openai".to_owned())],
        "the routed wrapper sends the provider it was given"
    );

    let mut capabilities = Capabilities::new(&[]);
    capabilities.providers.push("openai".to_owned());
    let mut unrouted = object(request_json);
    let folded = adapter()
        .translate_completion_input_params(&mut unrouted, &settings, &mut capabilities)
        .await
        .expect("the convenience request composes");
    assert_eq!(
        folded.get("messages"),
        Some(&json(&format!(
            r#"[{{"role": "user", "content": "hi"}},
                {{"role": "user", "content": [
                    {{"type": "text", "text": "{CONVERTED_SYSTEM_NOTE}"}},
                    {{"type": "text", "text": "late"}}
                ]}}]"#
        ))),
        "the convenience wrapper names no provider, so the trailing system turn is converted"
    );
    assert_eq!(
        capabilities.provider_probes,
        [None],
        "the convenience wrapper composes as if no provider was named"
    );
}

#[tokio::test]
async fn option_step_errors_surface_through_the_composition() {
    let mut kwargs = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": {"type": "bogus"}
        }"#,
    );
    let error = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect_err("an unknown tool_choice type is refused");
    assert!(matches!(
        error,
        TransformError::ToolChoice(ToolChoiceError::Unsupported)
    ));
}

#[tokio::test]
async fn composed_message_response_wraps_the_chat_completion() {
    let record: ResponseRecord = serde_json::from_value(json!({
        "id": "chat_a",
        "model": "gpt-4o",
        "choices": [{
            "finish_reason": "stop",
            "message": {"content": "hello"}
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2}
    }))
    .expect("the response record decodes");
    let message = adapter()
        .translate_completion_output_params(&record, None, None)
        .expect("the response translates");
    assert_eq!(
        serde_json::to_value(&message).expect("the message encodes"),
        json!({
            "id": "chat_a",
            "type": "message",
            "role": "assistant",
            "model": "gpt-4o",
            "stop_sequence": null,
            "usage": {"input_tokens": 10, "output_tokens": 2},
            "content": [{"type": "text", "text": "hello"}],
            "stop_reason": "end_turn",
            "stop_details": null
        })
    );
}

#[tokio::test]
async fn polyfill_output_reaches_the_message_response() {
    let record: ResponseRecord = serde_json::from_value(json!({
        "id": "chat_a",
        "model": "gpt-4o",
        "choices": [{
            "finish_reason": "stop",
            "message": {"content": "hello"}
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2}
    }))
    .expect("the response record decodes");
    let polyfill = PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: vec![AppliedEdit {
            edit_type: Some("clear_tool_uses_20250919".to_owned()),
            cleared_tool_uses: Some(1),
            ..AppliedEdit::default()
        }],
        compaction_block: Some(CompactionBlock {
            kind: CompactionKind::Compaction,
            content: Some("sum".to_owned()),
            signature: Some("s".to_owned()),
        }),
        iterations_usage: Some(vec![UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(4),
            output_tokens: Some(1),
        }]),
    };
    let message = adapter()
        .translate_completion_output_params(&record, None, Some(&polyfill))
        .expect("the response translates");
    let value = serde_json::to_value(&message).expect("the message encodes");
    assert_eq!(
        value["content"][0],
        json!({"type": "compaction", "content": "sum", "signature": "s"}),
        "the compaction block leads the content"
    );
    assert_eq!(
        value["context_management"]["applied_edits"],
        json!([{"type": "clear_tool_uses_20250919", "cleared_tool_uses": 1}])
    );
    assert_eq!(
        value["usage"]["iterations"],
        json!([
            {"type": "compaction", "input_tokens": 4, "output_tokens": 1},
            {"type": "message", "input_tokens": 10, "output_tokens": 2}
        ]),
        "the summary iteration comes before the message iteration"
    );
}

#[derive(Clone)]
struct Scripted {
    items: VecDeque<StreamChunk>,
    chunks: Vec<StreamChunk>,
}

impl ChunkSource for Scripted {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        let Some(chunk) = self.items.pop_front() else {
            return Ok(None);
        };
        self.chunks.push(chunk.clone());
        Ok(Some(chunk))
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        Some(&self.chunks)
    }
}

#[async_trait]
impl AsyncChunkSource for Scripted {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        let Some(chunk) = self.items.pop_front() else {
            return Ok(None);
        };
        self.chunks.push(chunk.clone());
        Ok(Some(chunk))
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        Some(&self.chunks)
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

fn chunk(source: &str) -> StreamChunk {
    serde_json::from_str(source).expect("a valid chunk fixture")
}

fn text(value: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{"content":"{value}"}},"finish_reason":null}}]}}"#
    ))
}

fn finish(reason: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#
    ))
}

fn usage_chunk(prompt: i64, completion: i64) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[],"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion}}}}}"#
    ))
}

fn tool_call(name: &str) -> StreamChunk {
    let call = json!({
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": name, "arguments": "{\"x\":1}"}
                }]
            },
            "finish_reason": null
        }]
    });
    serde_json::from_value(call).expect("a valid tool-call chunk")
}

fn scripted(chunks: Vec<StreamChunk>) -> Scripted {
    Scripted {
        items: chunks.into_iter().collect(),
        chunks: Vec::new(),
    }
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a valid JSON fixture")
}

// The frames as (event name, data), with the generated message id replaced by one value.
fn frames(frames: &[String], message_id: &str) -> Vec<(String, Json)> {
    frames
        .iter()
        .map(|frame| {
            let normalized = frame.replace(message_id, "msg_fixed");
            let mut lines = normalized.lines();
            let event = lines
                .next()
                .expect("an event line")
                .strip_prefix("event: ")
                .expect("the event prefix")
                .to_owned();
            let data = lines
                .next()
                .expect("a data line")
                .strip_prefix("data: ")
                .expect("the data prefix");
            (event, json(data))
        })
        .collect()
}

// The value under a path inside one event's data object; a test names a path it knows exists.
fn at<'events>(events: &'events [(String, Json)], index: usize, path: &[&str]) -> &'events Json {
    let mut current: &Json = &events[index].1;
    for key in path {
        current = current
            .as_object()
            .and_then(|fields| fields.get(*key))
            .expect("the event carries the asserted path");
    }
    current
}

async fn drain_async(view: &mut AsyncView) -> (Vec<String>, String) {
    let message_id = view.stream.state.message_id.clone();
    let mut collected = Vec::new();
    while let Some(frame) = view.next_frame().await {
        collected.push(String::from_utf8(frame).expect("the frame is text"));
    }
    (collected, message_id)
}

fn drain_sync(driver: SyncIterator) -> (Vec<String>, String) {
    let message_id = driver.driver.state.message_id.clone();
    let collected = driver
        .map(|frame| {
            String::from_utf8(frame.expect("the frame encodes")).expect("the frame is text")
        })
        .collect();
    (collected, message_id)
}

fn event_names(events: &[(String, Json)]) -> Vec<&str> {
    events.iter().map(|(name, _)| name.as_str()).collect()
}

fn text_stream() -> Scripted {
    scripted(vec![text("hello"), finish("stop"), usage_chunk(10, 2)])
}

#[tokio::test]
async fn stream_factory_defaults_to_the_async_view() {
    let output = adapter().translate_completion_output_params_streaming(
        text_stream(),
        StreamOptions::new("gpt-4o"),
        FixedIds,
    );
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let (collected, message_id) = drain_async(&mut view).await;
    let events = frames(&collected, &message_id);
    assert_eq!(
        event_names(&events),
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    assert_eq!(at(&events, 0, &["message", "model"]), &json(r#""gpt-4o""#));
    assert_eq!(at(&events, 0, &["message", "id"]), &json(r#""msg_fixed""#));
    assert_eq!(
        at(&events, 2, &["delta"]),
        &json(r#"{"type": "text_delta", "text": "hello"}"#)
    );
    assert_eq!(
        at(&events, 4, &["delta", "stop_reason"]),
        &json(r#""end_turn""#)
    );
    assert_eq!(at(&events, 4, &["usage", "input_tokens"]), &json("10"));
    assert_eq!(at(&events, 4, &["usage", "output_tokens"]), &json("2"));

    assert_eq!(view.model(), "gpt-4o");
    assert_eq!(
        view.chunks().map(<[StreamChunk]>::len),
        Some(3),
        "every provider chunk is recorded for partial billing"
    );
    assert!(
        view.messages().is_none(),
        "a scripted source exposes no chat messages"
    );
    view.aclose();
    assert!(
        view.next_frame().await.is_none(),
        "a closed view yields nothing"
    );
}

#[tokio::test]
async fn stream_factory_returns_a_sync_iterator_when_asked() {
    let mut options = StreamOptions::new("gpt-4o");
    options.is_async = false;
    let output =
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds);
    let StreamOutput::Sync(driver) = output else {
        panic!("is_async=false returns the sync iterator");
    };
    let (collected, message_id) = drain_sync(*driver);

    let async_output = adapter().translate_completion_output_params_streaming(
        text_stream(),
        StreamOptions::new("gpt-4o"),
        FixedIds,
    );
    let StreamOutput::Async(mut view) = async_output else {
        panic!("is_async defaults to true");
    };
    let (async_collected, async_id) = drain_async(&mut view).await;
    assert_eq!(
        frames(&collected, &message_id),
        frames(&async_collected, &async_id),
        "both modes emit the same events"
    );
}

#[tokio::test]
async fn stream_factory_restores_tool_names_through_the_mapping() {
    let long = "a".repeat(65);
    let short = format!("{}_635361c4", "a".repeat(55));
    let mapping = IndexMap::from([(short.clone(), long.clone())]);
    let mut options = StreamOptions::new("gpt-4o");
    options.tool_name_mapping = Some(&mapping);
    let output = adapter().translate_completion_output_params_streaming(
        scripted(vec![tool_call(&short), finish("tool_calls")]),
        options,
        FixedIds,
    );
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let (collected, message_id) = drain_async(&mut view).await;
    let events = frames(&collected, &message_id);
    assert_eq!(
        event_names(&events),
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    assert_eq!(
        at(&events, 1, &["content_block", "type"]),
        &json(r#""tool_use""#)
    );
    assert_eq!(
        at(&events, 1, &["content_block", "name"]),
        &json(&format!("\"{long}\""))
    );
    assert_eq!(
        at(&events, 4, &["delta", "stop_reason"]),
        &json(r#""tool_use""#)
    );
}

fn polyfill() -> PolyfillResult {
    PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: vec![AppliedEdit {
            edit_type: Some("clear_tool_uses_20250919".to_owned()),
            cleared_tool_uses: Some(1),
            ..AppliedEdit::default()
        }],
        compaction_block: Some(CompactionBlock {
            kind: CompactionKind::Compaction,
            content: Some("sum".to_owned()),
            signature: Some("s".to_owned()),
        }),
        iterations_usage: Some(vec![UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(4),
            output_tokens: Some(1),
        }]),
    }
}

fn polyfilled_events(events: &[(String, Json)]) {
    assert_eq!(
        at(events, 1, &["content_block", "type"]),
        &json(r#""compaction""#),
        "the compaction block opens before the text"
    );
    assert_eq!(
        at(events, 7, &["context_management", "applied_edits"]),
        &json(r#"[{"type": "clear_tool_uses_20250919", "cleared_tool_uses": 1}]"#)
    );
    assert_eq!(
        at(events, 7, &["usage", "iterations"]),
        &json(
            r#"[{"type": "compaction", "input_tokens": 4, "output_tokens": 1},
                {"type": "message", "input_tokens": 10, "output_tokens": 2}]"#
        ),
        "the summary iteration comes before the message iteration"
    );
}

#[tokio::test]
async fn stream_factory_passes_the_polyfill_output_to_both_modes() {
    let polyfill = polyfill();
    let mut options = StreamOptions::new("gpt-4o");
    options.polyfill_result = Some(&polyfill);
    let output =
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds);
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let (collected, message_id) = drain_async(&mut view).await;
    let events = frames(&collected, &message_id);
    assert_eq!(
        event_names(&events),
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    polyfilled_events(&events);

    let mut options = StreamOptions::new("gpt-4o");
    options.polyfill_result = Some(&polyfill);
    options.is_async = false;
    let output =
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds);
    let StreamOutput::Sync(driver) = output else {
        panic!("is_async=false returns the sync iterator");
    };
    let (collected, message_id) = drain_sync(*driver);
    polyfilled_events(&frames(&collected, &message_id));
}

#[tokio::test]
async fn stream_factory_records_the_message_id_on_the_callers_logging_state() {
    let mut logging = logging_state();
    let output = {
        let mut options = StreamOptions::new("gpt-4o");
        options.logging = Some(&mut logging);
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds)
    };
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let message_id = view.stream.state.message_id.clone();
    assert_eq!(
        logging.streamed_anthropic_message_id.as_deref(),
        Some(message_id.as_str()),
        "the caller's logging state received the stream's message id"
    );
    let (collected, _) = drain_async(&mut view).await;
    assert_eq!(
        at(&frames(&collected, &message_id), 0, &["message", "id"]),
        &json(r#""msg_fixed""#)
    );
}

fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .expect("the fixture time is RFC 3339"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}
