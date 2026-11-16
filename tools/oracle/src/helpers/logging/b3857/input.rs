use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    #[default]
    RecordSuccess,
    RequestInput,
    ResponseOutput,
    StreamOutput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseKind {
    ModelResponse,
    ResponsesApi,
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChunkKind {
    Chat,
    Completed,
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkInput {
    #[serde(rename = "type")]
    pub(crate) kind: ChunkKind,
    pub(crate) value: OtelModelValue,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) enable_events: Option<bool>,
    pub(crate) semconv: bool,
    pub(crate) capture_mode: Option<String>,
    pub(crate) attributes: Option<OtelModelValue>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) config: Config,
    pub(crate) callback_name: Option<String>,
    pub(crate) callback_settings: Option<OtelModelValue>,
    pub(crate) kwargs: MetadataMap,
    pub(crate) response: Option<OtelModelValue>,
    pub(crate) start_time: Option<OtelModelValue>,
    pub(crate) end_time: Option<OtelModelValue>,
    pub(crate) data: MetadataMap,
    pub(crate) kind: Option<ResponseKind>,
    pub(crate) chunks: Vec<ChunkInput>,
}
