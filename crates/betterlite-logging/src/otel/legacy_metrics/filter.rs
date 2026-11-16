use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use opentelemetry::KeyValue;
use serde::Deserialize;

use super::error::LegacyMetricsError;
use crate::otel::model::utils::coercion::OtelModelValue;

mod tests;

pub const TOKEN_TYPE_ATTRIBUTE: &str = "gen_ai.token.type";

pub const METRIC_METADATA_KEYS: [&str; 15] = [
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_org_id",
    "user_api_key_user_id",
    "user_api_key_team_alias",
    "user_api_key_user_email",
    "spend_logs_metadata",
    "requester_ip_address",
    "requester_metadata",
    "user_api_key_end_user_id",
    "prompt_management_metadata",
    "applied_guardrails",
    "mcp_tool_call_metadata",
    "vector_store_request_metadata",
];

const BASE_ATTRIBUTE_NAMES: [&str; 6] = [
    "gen_ai.operation.name",
    "gen_ai.provider.name",
    "gen_ai.system",
    "gen_ai.request.model",
    "gen_ai.framework",
    "hidden_params",
];

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct MetricAttributeFilter {
    pub include_list: Option<Vec<String>>,
    pub exclude_list: Option<Vec<String>>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ResolvedFilter {
    pub include: Option<BTreeSet<String>>,
    pub exclude: Option<BTreeSet<String>>,
}

// the proxy fills callback_settings.otel.attributes after the logger exists, so it is read on the first record
pub trait CallbackSettingsView: Send + Sync {
    fn otel_attributes(&self) -> Option<OtelModelValue>;
}

pub struct MetricFilterState {
    config_attributes: Option<OtelModelValue>,
    callback_name: Option<String>,
    callback_settings: Arc<dyn CallbackSettingsView>,
    resolved: Mutex<Option<Arc<ResolvedFilter>>>,
}

pub fn build_metric_attribute_filter(
    value: &OtelModelValue,
) -> Result<MetricAttributeFilter, LegacyMetricsError> {
    if value.as_object().is_none() {
        return Err(LegacyMetricsError::AttributesNotMapping);
    }
    let json = serde_json::to_value(value)
        .map_err(|source| LegacyMetricsError::AttributesShape { source })?;
    serde_json::from_value(json).map_err(|source| LegacyMetricsError::AttributesShape { source })
}

fn is_valid_name(name: &str) -> bool {
    BASE_ATTRIBUTE_NAMES.contains(&name)
        || name
            .strip_prefix("metadata.")
            .is_some_and(|key| METRIC_METADATA_KEYS.contains(&key))
}

pub fn resolve_metric_filter(
    configured: Option<&MetricAttributeFilter>,
) -> Result<ResolvedFilter, LegacyMetricsError> {
    let Some(lists) = configured else {
        return Ok(ResolvedFilter::default());
    };
    let include = lists
        .include_list
        .as_deref()
        .filter(|names| !names.is_empty());
    let exclude = lists
        .exclude_list
        .as_deref()
        .filter(|names| !names.is_empty());
    if include.is_some() && exclude.is_some() {
        return Err(LegacyMetricsError::BothLists);
    }
    let requested = include.or(exclude).unwrap_or_default();
    if requested.iter().any(|name| name == TOKEN_TYPE_ATTRIBUTE) {
        return Err(LegacyMetricsError::TokenTypeFiltered);
    }
    let mut names: Vec<String> = requested
        .iter()
        .filter(|name| !is_valid_name(name))
        .cloned()
        .collect();
    names.sort();
    if !names.is_empty() {
        return Err(LegacyMetricsError::UnknownAttributeNames { names });
    }
    Ok(ResolvedFilter {
        include: include.map(|listed| listed.iter().cloned().collect()),
        exclude: exclude.map(|listed| listed.iter().cloned().collect()),
    })
}

impl ResolvedFilter {
    #[must_use]
    pub fn allows(&self, key: &str) -> bool {
        if let Some(include) = &self.include {
            return include.contains(key);
        }
        if let Some(exclude) = &self.exclude {
            return !exclude.contains(key);
        }
        true
    }
}

impl MetricFilterState {
    // a configured filter is validated here so a bad otel.attributes fails at startup, not on the first request
    pub fn new(
        config_attributes: Option<OtelModelValue>,
        callback_name: Option<String>,
        callback_settings: Arc<dyn CallbackSettingsView>,
    ) -> Result<Self, LegacyMetricsError> {
        let state = Self {
            config_attributes: config_attributes
                .filter(|value| !matches!(value, OtelModelValue::Null(()))),
            callback_name,
            callback_settings,
            resolved: Mutex::new(None),
        };
        if state.config_attributes.is_some() {
            state.ensure()?;
        }
        Ok(state)
    }

    pub fn ensure(&self) -> Result<Arc<ResolvedFilter>, LegacyMetricsError> {
        let cached = self
            .resolved
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(resolved) = cached {
            return Ok(resolved);
        }
        let mut attributes = self.config_attributes.clone();
        if attributes.is_none() && matches!(self.callback_name.as_deref(), None | Some("otel")) {
            attributes = self
                .callback_settings
                .otel_attributes()
                .filter(|value| !matches!(value, OtelModelValue::Null(())));
        }
        let filter = attributes
            .as_ref()
            .map(build_metric_attribute_filter)
            .transpose()?;
        let resolved = Arc::new(resolve_metric_filter(filter.as_ref())?);
        *self.resolved.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&resolved));
        Ok(resolved)
    }

    pub fn filter_attributes(
        &self,
        attributes: Vec<(String, Option<String>)>,
    ) -> Result<Vec<KeyValue>, LegacyMetricsError> {
        let resolved = self.ensure()?;
        Ok(attributes
            .into_iter()
            .filter_map(|(key, value)| {
                let text = value.filter(|_| resolved.allows(&key))?;
                Some(KeyValue::new(key, text))
            })
            .collect())
    }
}
