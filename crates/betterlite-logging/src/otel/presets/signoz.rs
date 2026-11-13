use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_http::url_policy::destination::destination_allowed;

use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::presets::base::{Preset, PresetError};
use crate::otel::presets::utils::ensure_mappers;

pub const SIGNOZ_INGESTION_ENDPOINT_ENV: &str = "SIGNOZ_INGESTION_ENDPOINT";
const INGESTION_KEY_ENV: &str = "SIGNOZ_INGESTION_KEY";
const ENDPOINT_FIELD: &str = "signoz_ingestion_endpoint";
const KEY_FIELD: &str = "signoz_ingestion_key";

pub struct SignozPreset;

impl Preset for SignozPreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        _allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        // The endpoint is passed through as configured; only the legacy path appends /v1/traces
        let endpoint = environment.lookup(SIGNOZ_INGESTION_ENDPOINT_ENV);
        let key = environment
            .lookup(INGESTION_KEY_ENV)
            .filter(|value| !value.is_empty());
        let mut settings = base;
        settings.exporters.push(ExporterSpec {
            kind: "otlp_http".to_owned(),
            endpoint,
            traces_endpoint: None,
            headers: key
                .as_ref()
                .map(|ingestion_key| format!("signoz-ingestion-key={ingestion_key}")),
            owner: Some(ExporterOwner::Signoz),
            options: None,
            use_simple_processor: None,
            requires_headers: key.is_some(),
        });
        settings.mapper_names = ensure_mappers(&settings.mapper_names, &["genai"]);
        Ok(settings)
    }
}

// A key/team endpoint is only used when it carries a scheme, its own key, and an allowlisted host
#[must_use]
pub fn signoz_dynamic_endpoint(
    params: &BTreeMap<String, String>,
    allowed_hosts: &[AllowedDestination],
) -> Option<String> {
    let endpoint = params
        .get(ENDPOINT_FIELD)
        .filter(|value| !value.is_empty())?;
    if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
        return None;
    }
    if params.get(KEY_FIELD).is_none_or(String::is_empty) {
        tracing::warn!(
            "SigNoz: not exporting to key/team endpoint '{endpoint}'. Set signoz_ingestion_key alongside it; a keyless collector needs the global callback"
        );
        return None;
    }
    if !destination_allowed(endpoint, allowed_hosts) {
        tracing::warn!(
            "SigNoz: not exporting to key/team endpoint '{endpoint}'. Add its host to litellm_settings.provider_url_destination_allowed_hosts to permit it"
        );
        return None;
    }
    Some(endpoint.clone())
}

// A tenant endpoint that is set but unusable also empties the headers, so the key never reaches the operator's collector
#[must_use]
pub fn signoz_dynamic_headers(
    params: &BTreeMap<String, String>,
    allowed_hosts: &[AllowedDestination],
) -> BTreeMap<String, String> {
    let Some(key) = params.get(KEY_FIELD).filter(|value| !value.is_empty()) else {
        return BTreeMap::new();
    };
    let tenant_endpoint = params.get(ENDPOINT_FIELD).filter(|value| !value.is_empty());
    if tenant_endpoint.is_some() && signoz_dynamic_endpoint(params, allowed_hosts).is_none() {
        return BTreeMap::new();
    }
    BTreeMap::from([("signoz-ingestion-key".to_owned(), key.clone())])
}

mod tests;
