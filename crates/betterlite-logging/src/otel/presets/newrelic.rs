use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_env::flag;

use crate::otel::config::{CaptureMessageContent, ExporterOwner, ExporterSpec, Settings};
use crate::otel::presets::base::{Preset, PresetError};
use crate::otel::presets::utils::ensure_mappers;

const US_ENDPOINT: &str = "https://otlp.nr-data.net";
const EU_ENDPOINT: &str = "https://otlp.eu01.nr-data.net";
const LICENSE_KEY_ENV: &str = "NEW_RELIC_LICENSE_KEY";
const REGION_ENV: &str = "NEW_RELIC_REGION";
const RECORD_CONTENT_ENV: &str = "NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED";
const DEFAULT_REGION: &str = "us";

#[derive(Debug, thiserror::Error)]
pub enum NewrelicPresetError {
    #[error("NEW_RELIC_AI_MONITORING_RECORD_CONTENT_ENABLED must be a bool, got {value:?}")]
    RecordContent { value: String },
}

// The table is a fixed region-to-endpoint map, so team config can never redirect telemetry to an arbitrary host
#[must_use]
pub fn newrelic_endpoint_for_region(region: &str) -> Option<&'static str> {
    match region.to_ascii_lowercase().as_str() {
        "us" => Some(US_ENDPOINT),
        "eu" => Some(EU_ENDPOINT),
        _ => None,
    }
}

pub struct NewrelicPreset;

impl Preset for NewrelicPreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        _allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        let license_key = environment
            .lookup(LICENSE_KEY_ENV)
            .filter(|value| !value.is_empty());
        let region = environment
            .lookup(REGION_ENV)
            .unwrap_or_else(|| DEFAULT_REGION.to_owned());
        // An unknown operator region is silent here; only the tenant endpoint resolver warns
        let endpoint = newrelic_endpoint_for_region(&region).unwrap_or(US_ENDPOINT);
        let mut settings = base;
        settings.exporters.push(ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint: Some(endpoint.to_owned()),
            traces_endpoint: None,
            headers: license_key.map(|key| format!("api-key={key}")),
            owner: Some(ExporterOwner::Newrelic),
            options: None,
            use_simple_processor: None,
            requires_headers: true,
        });
        settings.mapper_names = ensure_mappers(&settings.mapper_names, &["genai"]);
        if let Some(value) = environment.lookup(RECORD_CONTENT_ENV) {
            settings.capture_message_content = if record_content(&value)? {
                CaptureMessageContent::SpanOnly
            } else {
                CaptureMessageContent::NoContent
            };
        }
        Ok(settings)
    }
}

// pydantic reads a bool env value without trimming, so surrounding whitespace is as invalid as an unknown word
fn record_content(value: &str) -> Result<bool, NewrelicPresetError> {
    if value.trim() != value {
        return Err(NewrelicPresetError::RecordContent {
            value: value.to_owned(),
        });
    }
    match value.to_ascii_lowercase().as_str() {
        "t" | "y" => Ok(true),
        "f" | "n" => Ok(false),
        other => flag(RECORD_CONTENT_ENV, other)
            .map_err(|_source| NewrelicPresetError::RecordContent {
                value: value.to_owned(),
            })?
            .ok_or_else(|| NewrelicPresetError::RecordContent {
                value: value.to_owned(),
            }),
    }
}

#[must_use]
pub fn newrelic_dynamic_headers(params: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    if let Some(api_key) = params
        .get("newrelic_api_key")
        .filter(|value| !value.is_empty())
    {
        headers.insert("api-key".to_owned(), api_key.clone());
    }
    headers
}

// A team that saved only its ingest key must not inherit the operator's NEW_RELIC_REGION
#[must_use]
pub fn newrelic_dynamic_endpoint(params: &BTreeMap<String, String>) -> String {
    let Some(region) = params
        .get("newrelic_region")
        .filter(|value| !value.is_empty())
    else {
        return US_ENDPOINT.to_owned();
    };
    let Some(endpoint) = newrelic_endpoint_for_region(region) else {
        tracing::warn!(
            "New Relic: unknown newrelic_region '{region}'; supported regions: eu, us. Using the default (US) endpoint."
        );
        return US_ENDPOINT.to_owned();
    };
    endpoint.to_owned()
}

mod tests;
