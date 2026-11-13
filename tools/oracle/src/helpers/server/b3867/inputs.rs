use std::collections::BTreeMap;

use betterlite_logging::otlp_normalize::SpanRow;
use betterlite_logging::trace_types::{SpanDetail, SpanErrorPage, Trace, TracePage};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) request: RequestInput,
    #[serde(default)]
    pub(crate) auth: AuthInput,
    #[serde(default = "default_tracing")]
    pub(crate) tracing: bool,
    #[serde(default)]
    pub(crate) master_key: Option<String>,
    #[serde(default)]
    pub(crate) script: Box<Script>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    pub(crate) method: String,
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) body: Option<BodyInput>,
    #[serde(default)]
    pub(crate) query: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyInput {
    pub(crate) encoding: Encoding,
    pub(crate) value: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Encoding {
    Utf8,
    Base64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthInput {
    #[serde(default)]
    pub(crate) user_role: Option<String>,
    #[serde(default)]
    pub(crate) team_id: Option<String>,
    #[serde(default)]
    pub(crate) token: Option<String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) org_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Script {
    #[serde(default)]
    pub(crate) decoder: DecoderScript,
    #[serde(default)]
    pub(crate) now_ms: Option<i64>,
    #[serde(default)]
    pub(crate) list_traces: ReadScript<TracePage>,
    #[serde(default)]
    pub(crate) get_trace: ReadScript<Trace>,
    #[serde(default)]
    pub(crate) get_span: ReadScript<SpanDetail>,
    #[serde(default)]
    pub(crate) get_span_error: ReadScript<SpanErrorPage>,
    #[serde(default)]
    pub(crate) query: QueryScript,
    #[serde(default)]
    pub(crate) query_help: QueryScript,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecoderScript {
    #[serde(default)]
    pub(crate) kind: DecoderKind,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) rows: Vec<SpanRow>,
}

impl Default for DecoderScript {
    fn default() -> Self {
        Self {
            kind: DecoderKind::Ok,
            message: None,
            rows: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecoderKind {
    #[default]
    Ok,
    TooLarge,
    Overflow,
    Malformed,
    Overloaded,
}

#[derive(Debug, Clone)]
pub(crate) struct ReadScript<Payload> {
    pub(crate) value: Option<Payload>,
    pub(crate) error: Option<ErrorScript>,
}

impl<Payload> Default for ReadScript<Payload> {
    fn default() -> Self {
        Self {
            value: None,
            error: None,
        }
    }
}

impl<'de, Payload> Deserialize<'de> for ReadScript<Payload>
where
    Payload: Deserialize<'de>,
{
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let raw = RawReadScript::<Payload>::deserialize(deserializer)?;
        Ok(Self {
            value: raw.value,
            error: raw.error,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReadScript<Payload> {
    #[serde(default)]
    value: Option<Payload>,
    #[serde(default)]
    error: Option<ErrorScript>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueryScript {
    #[serde(default)]
    pub(crate) value: Option<String>,
    #[serde(default)]
    pub(crate) error: Option<ErrorScript>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorScript {
    pub(crate) kind: ErrorKind,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    InvalidCursor,
    Storage,
    Invalid,
    Unavailable,
}

fn default_tracing() -> bool {
    true
}
