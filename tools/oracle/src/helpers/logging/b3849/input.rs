use std::collections::BTreeMap;

use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) function: String,
    #[serde(default)]
    pub(crate) span: SpanInput,
    pub(crate) endpoint: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanInput {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) attributes: BTreeMap<String, OtelModelValue>,
}
