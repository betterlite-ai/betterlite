use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_translation::bedrock_streaming::BedrockGuardrailStreamParams;
use betterlite_core::routes::CallType;
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;

use super::fake::FakeRuntime;
use crate::guardrail_hooks::StreamingSettings;
use crate::guardrail_hooks::chunks::StreamChunk;
use crate::guardrail_hooks::errors::HookFailure;
use crate::guardrail_hooks::streaming::{StreamHook, streaming_iterator_hook};

pub(crate) async fn run_buffered(
    runtime: &FakeRuntime,
    settings: &StreamingSettings,
    request_route: &str,
    chunks: Vec<StreamChunk>,
) -> Result<Vec<StreamChunk>, HookFailure> {
    let table = FixedRoutes {
        route: "/v1/chat/completions",
        call_types: vec![CallType::Completion],
    };
    let mut data = BTreeMap::new();
    streaming_iterator_hook(StreamHook {
        runtime,
        settings,
        routes: &table,
        request_route: Some(request_route),
        data: &mut data,
        chunks,
    })
    .await
}

pub(crate) struct FixedRoutes {
    pub route: &'static str,
    pub call_types: Vec<CallType>,
}

impl RouteCallTypes<CallType> for FixedRoutes {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<CallType>> {
        (route == self.route).then_some(self.call_types.clone())
    }
}

pub(crate) fn routes(route: &'static str, call_types: Vec<CallType>) -> FixedRoutes {
    FixedRoutes { route, call_types }
}

pub(crate) fn streaming_settings() -> StreamingSettings {
    StreamingSettings {
        guardrail_name: "bedrock-guard".to_owned(),
        params: BedrockGuardrailStreamParams::default(),
        mask_response_content: false,
        keepalive_interval_seconds: None,
    }
}

pub(crate) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(crate) fn unsigned(value: u64) -> GuardrailValue {
    GuardrailValue::UnsignedInteger(value)
}

pub(crate) fn object(fields: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    GuardrailValue::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(crate) fn array(items: Vec<GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Array(items)
}

pub(crate) fn message(role: &str, content: GuardrailValue) -> GuardrailValue {
    object(vec![("role", text(role)), ("content", content)])
}

pub(crate) fn usage(prompt_tokens: u64, completion_tokens: u64) -> GuardrailValue {
    object(vec![
        ("prompt_tokens", unsigned(prompt_tokens)),
        ("completion_tokens", unsigned(completion_tokens)),
        (
            "total_tokens",
            unsigned(prompt_tokens.saturating_add(completion_tokens)),
        ),
    ])
}

pub(crate) fn chat_response(model: &str, content: GuardrailValue) -> GuardrailValue {
    object(vec![
        ("usage", usage(3, 2)),
        (
            "choices",
            array(vec![object(vec![
                ("message", message("assistant", content)),
                ("index", unsigned(0)),
                ("finish_reason", text("stop")),
            ])]),
        ),
        ("model", text(model)),
        ("created", unsigned(1)),
        ("object", text("chat.completion")),
        ("id", text("chatcmpl-1")),
    ])
}

pub(crate) fn stream_chunk(content: &str) -> GuardrailValue {
    chunk(content, GuardrailValue::Null, false)
}

pub(crate) fn replay_chunk(content: &str) -> GuardrailValue {
    chunk(content, text("stop"), true)
}

fn chunk(content: &str, finish_reason: GuardrailValue, with_usage: bool) -> GuardrailValue {
    let mut choice = BTreeMap::new();
    choice.insert(
        "delta".to_owned(),
        object(vec![
            ("role", text("assistant")),
            ("content", text(content)),
        ]),
    );
    choice.insert("index".to_owned(), unsigned(0));
    choice.insert("finish_reason".to_owned(), finish_reason);
    let mut fields = BTreeMap::new();
    fields.insert(
        "choices".to_owned(),
        array(vec![GuardrailValue::Object(choice)]),
    );
    fields.insert("model".to_owned(), text("gpt-test"));
    fields.insert("id".to_owned(), text("chatcmpl-1"));
    fields.insert("object".to_owned(), text("chat.completion.chunk"));
    fields.insert("created".to_owned(), unsigned(1));
    if with_usage {
        fields.insert("usage".to_owned(), usage(3, 2));
    }
    GuardrailValue::Object(fields)
}

pub(crate) fn sse_frame(event: &str, data: &str) -> StreamChunk {
    StreamChunk::Bytes(format!("event: {event}\ndata: {data}\n\n").into_bytes())
}

pub(crate) fn frame_texts(chunks: &[StreamChunk]) -> Vec<String> {
    chunks
        .iter()
        .map(|chunk| match chunk {
            StreamChunk::Bytes(bytes) => String::from_utf8(bytes.clone()).expect("utf-8 frame"),
            StreamChunk::Json(value) => panic!("expected bytes, got {value:?}"),
        })
        .collect()
}
