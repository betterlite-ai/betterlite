use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: String,
    pub(crate) kwargs: MetadataMap,
    pub(crate) callback_name: Option<CallbackNames>,
    pub(crate) capture_mode: Option<String>,
    pub(crate) semconv: bool,
    pub(crate) baggage_metadata_keys: Vec<String>,
    pub(crate) baggage_team_metadata_keys: Vec<String>,
    pub(crate) response: Option<OtelModelValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum CallbackNames {
    One(String),
    Many(Vec<String>),
}
