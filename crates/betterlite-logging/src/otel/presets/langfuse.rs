use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use crate::langfuse_otel::destination::{
    build_langfuse_otel_headers, get_langfuse_otel_config, langfuse_authorization_header,
};
use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::presets::base::{Preset, PresetError};
use crate::otel::presets::utils::{credential_gated_exporters, ensure_mappers};

const LANGFUSE_CREDENTIAL_NAMES: [&str; 4] = [
    "LANGFUSE_HOST",
    "LANGFUSE_OTEL_HOST",
    "LANGFUSE_PUBLIC_KEY",
    "LANGFUSE_SECRET_KEY",
];

pub struct LangfusePreset;

impl Preset for LangfusePreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        let mappers = ensure_mappers(&base.mapper_names, &["langfuse"]);
        let mut settings = base;
        match get_langfuse_otel_config(&langfuse_environment(environment)) {
            Ok(config) => {
                settings.exporters.push(ExporterSpec {
                    kind: "otlp_http".to_owned(),
                    endpoint: config.endpoint,
                    traces_endpoint: None,
                    headers: config.headers,
                    owner: Some(ExporterOwner::LangfuseOtel),
                    options: None,
                    use_simple_processor: None,
                    requires_headers: false,
                });
                settings.mapper_names = mappers;
                Ok(settings)
            }
            Err(_) if allow_missing_credentials => {
                settings.exporters =
                    credential_gated_exporters(settings.exporters, ExporterOwner::LangfuseOtel);
                settings.mapper_names = mappers;
                Ok(settings)
            }
            Err(error) => Err(PresetError::Langfuse(error)),
        }
    }
}

// The Langfuse config helpers read a snapshot, so only the names they consume cross the lookup boundary
fn langfuse_environment(environment: &dyn EnvironmentLookup) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for name in LANGFUSE_CREDENTIAL_NAMES {
        if let Some(value) = environment.lookup(name) {
            values.insert(name.to_owned(), value);
        }
    }
    values
}

#[must_use]
pub fn langfuse_dynamic_headers(params: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let public_key = params
        .get("langfuse_public_key")
        .filter(|value| !value.is_empty());
    let secret_key = params
        .get("langfuse_secret_key")
        .filter(|value| !value.is_empty());
    let (Some(public), Some(secret)) = (public_key, secret_key) else {
        return BTreeMap::new();
    };
    let secret_value = SecretString::from(secret.clone());
    let auth_header = langfuse_authorization_header(public, &secret_value);
    build_langfuse_otel_headers(auth_header)
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.expose_secret().to_owned()))
        .collect()
}

mod tests;
