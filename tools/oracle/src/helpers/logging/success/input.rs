use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) operations: Vec<Operation>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operation {
    Assembled { probes: Vec<AssembledProbe> },
    Recognized { probes: Vec<RecognizedProbe> },
    Dispatch { probes: Vec<DispatchProbe> },
    Handler { probes: Vec<HandlerProbe> },
    SyncCallbacks { probes: Vec<SyncProbe> },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResultKind {
    Chat,
    Embeddings,
    Chunk,
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResultSpec {
    pub(crate) kind: ResultKind,
    #[serde(default)]
    pub(crate) value: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssembledProbe {
    pub(crate) stream: bool,
    pub(crate) result: ResultSpec,
    // "null" stores a null under the key; any other text stores that text; absent stores nothing
    #[serde(default)]
    pub(crate) async_key: Option<String>,
    #[serde(default)]
    pub(crate) complete_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecognizedProbe {
    pub(crate) result: ResultSpec,
    pub(crate) call_type: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LoggerSpec {
    pub(crate) name: String,
    #[serde(default, flatten)]
    pub(crate) flags: MessageFlagsSpec,
    #[serde(default)]
    pub(crate) fail: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct MessageFlagsSpec {
    #[serde(default)]
    pub(crate) turn_off_message_logging: bool,
    #[serde(default)]
    pub(crate) redacts_messages_itself: bool,
    pub(crate) message_logging: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Sync,
    Async,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextSpec {
    #[serde(default)]
    pub(crate) trace_id: String,
    #[serde(default)]
    pub(crate) session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchProbe {
    #[serde(default)]
    pub(crate) stream: bool,
    #[serde(default = "default_call_type")]
    pub(crate) call_type: String,
    #[serde(default)]
    pub(crate) litellm_params: BTreeMap<String, Value>,
    pub(crate) result: ResultSpec,
    #[serde(default)]
    pub(crate) prefer_async_handlers: bool,
    #[serde(default = "default_dispatches")]
    pub(crate) dispatches: u32,
    #[serde(default)]
    pub(crate) marker: Option<bool>,
    #[serde(default)]
    pub(crate) built_payload: Option<Value>,
    #[serde(default)]
    pub(crate) loggers: Vec<LoggerSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandlerProbe {
    pub(crate) mode: Mode,
    #[serde(default)]
    pub(crate) stream: bool,
    #[serde(default = "default_call_type")]
    pub(crate) call_type: String,
    #[serde(default)]
    pub(crate) litellm_params: BTreeMap<String, Value>,
    pub(crate) result: ResultSpec,
    pub(crate) start_time: String,
    pub(crate) end_time: String,
    #[serde(default)]
    pub(crate) cache_hit: Option<bool>,
    #[serde(default)]
    pub(crate) payload: Option<Value>,
    #[serde(default)]
    pub(crate) built_payload: Option<Value>,
    #[serde(default)]
    pub(crate) complete_response: Option<Value>,
    #[serde(default)]
    pub(crate) loggers: Vec<LoggerSpec>,
    #[serde(default)]
    pub(crate) excluded_fields: Vec<String>,
    #[serde(default)]
    pub(crate) stamp: Option<StampSpec>,
    #[serde(default)]
    pub(crate) context: ContextSpec,
    #[serde(default)]
    pub(crate) trace_id: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) request_correlation: bool,
    #[serde(default)]
    pub(crate) dynamic_params: BTreeMap<String, Value>,
    #[serde(default)]
    pub(crate) global_redaction: Option<bool>,
    #[serde(default)]
    pub(crate) assembler_returns_none: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StampSpec {
    pub(crate) id: String,
    pub(crate) created: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SyncProbe {
    #[serde(default)]
    pub(crate) dynamic_success: Vec<String>,
    #[serde(default)]
    pub(crate) known: Vec<String>,
    #[serde(default)]
    pub(crate) loggers: Vec<LoggerSpec>,
}

fn default_call_type() -> String {
    "completion".to_owned()
}

const fn default_dispatches() -> u32 {
    1
}
