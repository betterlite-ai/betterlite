use std::collections::BTreeMap;

use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Input {
    pub(crate) function: String,
    #[serde(default)]
    pub(crate) span: SpanInput,
    #[serde(default)]
    pub(crate) destination: DestinationInput,
    pub(crate) span_scope: Option<String>,
    pub(crate) endpoint: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) excluded: Vec<String>,
    #[serde(default)]
    pub(crate) header: String,
}

#[derive(Deserialize, Default)]
pub(crate) struct SpanInput {
    #[serde(default = "default_name")]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) attributes: BTreeMap<String, OtelModelValue>,
    #[serde(default)]
    pub(crate) events: Vec<EventInput>,
    #[serde(default)]
    pub(crate) status: StatusInput,
    pub(crate) parent: Option<String>,
    #[serde(default)]
    pub(crate) resource: BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct EventInput {
    #[serde(default = "default_event_name")]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) attributes: BTreeMap<String, OtelModelValue>,
}

#[derive(Deserialize)]
pub(crate) struct StatusInput {
    pub(crate) code: String,
    pub(crate) description: Option<String>,
}

impl Default for StatusInput {
    fn default() -> Self {
        Self {
            code: "unset".to_owned(),
            description: None,
        }
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct DestinationInput {
    pub(crate) endpoint: String,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) resource_attributes: BTreeMap<String, String>,
    pub(crate) callback_name: Option<String>,
    pub(crate) protocol: Option<String>,
    pub(crate) span_scope: Option<String>,
}

fn default_name() -> String {
    "call model".to_owned()
}

fn default_event_name() -> String {
    "exception".to_owned()
}
