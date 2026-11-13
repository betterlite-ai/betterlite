use std::collections::BTreeMap;

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use percent_encoding::percent_decode_str;
use tracing::warn;

use crate::otel::legacy_policy::config::OpenTelemetryConfig;

const SERVICE_NAME: &str = "service.name";
const RESOURCE_ATTRIBUTES_ENV: &str = "OTEL_RESOURCE_ATTRIBUTES";
const SERVICE_NAME_ENV: &str = "OTEL_SERVICE_NAME";

// The SDK's OTELResourceDetector reads the same two variables, values percent-decoded and names trimmed.
fn env_attributes(env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    let raw = env.get(RESOURCE_ATTRIBUTES_ENV).map_or("", String::as_str);
    for (index, item) in raw.split(',').enumerate() {
        if item.is_empty() {
            continue;
        }
        let Some((name, value)) = item
            .split_once('=')
            .filter(|(name, _value)| !name.trim().is_empty())
        else {
            warn!(
                position = index.saturating_add(1),
                "otel: dropping an OTEL_RESOURCE_ATTRIBUTES entry that is not name=value"
            );
            continue;
        };
        let decoded = percent_decode_str(value.trim()).decode_utf8_lossy();
        attributes.insert(name.trim().to_owned(), decoded.into_owned());
    }
    if let Some(name) = env.get(SERVICE_NAME_ENV).filter(|value| !value.is_empty()) {
        attributes.insert(SERVICE_NAME.to_owned(), name.clone());
    }
    attributes
}

#[must_use]
pub fn legacy_resource(config: &OpenTelemetryConfig, env: &BTreeMap<String, String>) -> Resource {
    let mut attributes = BTreeMap::new();
    if !config.service_name.is_empty() {
        attributes.insert(SERVICE_NAME.to_owned(), config.service_name.clone());
    }
    if !config.deployment_environment.is_empty() {
        attributes.insert(
            "deployment.environment".to_owned(),
            config.deployment_environment.clone(),
        );
    }
    let model_id = if config.model_id.is_empty() {
        config.service_name.as_str()
    } else {
        config.model_id.as_str()
    };
    if !model_id.is_empty() {
        attributes.insert("model_id".to_owned(), model_id.to_owned());
    }
    attributes.extend(env_attributes(env));
    Resource::builder_empty()
        .with_attributes(
            attributes
                .into_iter()
                .map(|(name, value)| KeyValue::new(name, value)),
        )
        .build()
}

#[path = "resource/tests.rs"]
mod tests;
