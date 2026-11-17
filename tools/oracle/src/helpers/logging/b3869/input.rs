use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    #[default]
    Record,
    RecordFailure,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: Function,
    pub(crate) callback_name: Option<String>,
    pub(crate) callback_settings: Option<OtelModelValue>,
    pub(crate) kwargs: MetadataMap,
    pub(crate) response: Option<OtelModelValue>,
    pub(crate) start_time: Option<OtelModelValue>,
    pub(crate) end_time: Option<OtelModelValue>,
}
