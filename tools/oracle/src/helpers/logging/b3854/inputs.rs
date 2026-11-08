use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_policy::dedupe::{HandlerIdentity, ScopeValue};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub(super) type ModelMap = BTreeMap<String, OtelModelValue>;

#[derive(Deserialize)]
pub(crate) struct EmitCall {
    pub(super) handler: HandlerIdentity,
    #[serde(default)]
    pub(super) scope: Vec<ScopeValue>,
}

#[derive(Deserialize)]
pub(crate) struct TeamInput {
    pub(super) team_id: Option<String>,
    pub(super) team_alias: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SpanNameInput {
    #[serde(default)]
    pub(super) litellm_params: ModelMap,
    pub(super) call_type: Option<String>,
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) semconv: bool,
}

#[derive(Deserialize)]
pub(crate) struct IdentityInput {
    pub(super) payload: ModelMap,
    #[serde(default)]
    pub(super) litellm_params: ModelMap,
    #[serde(default)]
    pub(super) baggage_team_metadata_keys: Vec<String>,
    #[serde(default)]
    pub(super) baggage_metadata_keys: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum MetadataInput {
    Text(String),
    List(Vec<String>),
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TimeInput {
    Seconds(f64),
    DateTime(DateTime<Utc>),
}
