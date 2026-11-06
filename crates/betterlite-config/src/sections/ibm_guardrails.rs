use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use super::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use super::guardrail_identity::item::GuardrailValue;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::tls::Verification;

mod tests;

pub const UI_FRIENDLY_NAME: &str = "IBM Guardrails Detector";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    guardrails: Vec<Entry>,
}

#[derive(Debug, Default, Deserialize)]
struct Entry {
    guardrail_name: String,
    litellm_params: Parameters,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Parameters {
    guardrail: String,
    auth_token: Option<SecretString>,
    base_url: Option<String>,
    detector_id: Option<String>,
    is_detector_server: Option<bool>,
    verify_ssl: Option<bool>,
    optional_params: Option<OptionalParameters>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OptionalParameters {
    detector_params: Option<BTreeMap<String, GuardrailValue>>,
    extra_headers: Option<BTreeMap<String, String>>,
    score_threshold: Option<f64>,
    block_on_detection: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DetectorOptionalParams {
    pub detector_params: Option<BTreeMap<String, GuardrailValue>>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub score_threshold: Option<f64>,
    pub block_on_detection: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DetectorConfigModel {
    pub auth_token: Option<SecretString>,
    pub base_url: Option<String>,
    pub detector_id: Option<String>,
    pub is_detector_server: Option<bool>,
    pub verify_ssl: Option<bool>,
    pub optional_params: Option<DetectorOptionalParams>,
}

impl GuardrailConfigModelIdentity for DetectorConfigModel {
    fn ui_friendly_name() -> &'static str {
        UI_FRIENDLY_NAME
    }
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub guardrails: Vec<Configured>,
}

#[derive(Debug, Clone)]
pub struct Configured {
    pub guardrail_name: String,
    pub auth_token: Option<SecretString>,
    pub base_url: Option<String>,
    pub detector_id: Option<String>,
    pub is_detector_server: bool,
    pub detector_params: BTreeMap<String, GuardrailValue>,
    pub extra_headers: BTreeMap<String, String>,
    pub score_threshold: Option<f64>,
    pub block_on_detection: bool,
    pub ssl_verification: Verification,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut guardrails = Vec::new();
        for (index, entry) in self.guardrails.into_iter().enumerate() {
            if entry.litellm_params.guardrail == "ibm_guardrails" {
                guardrails.push(entry.resolve(index, lookup)?);
            }
        }
        Ok(Settings { guardrails })
    }
}

impl Entry {
    fn resolve<L>(self, index: usize, lookup: &L) -> Result<Configured, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let prefix = format!("guardrails.{index}.litellm_params");
        let parameters = self.litellm_params;
        let token_path = format!("{prefix}.auth_token");
        let supplied = parameters
            .auth_token
            .map(|token| token.expose_secret().to_owned());
        let from_environment = lookup
            .lookup("IBM_GUARDRAILS_AUTH_TOKEN")
            .filter(|token| !token.is_empty());
        let auth_token = supplied
            .or(from_environment)
            .map(|token| resolve_string(token, &token_path, lookup))
            .transpose()?
            .map(SecretString::from);
        let optional = parameters.optional_params.unwrap_or_default();
        Ok(Configured {
            guardrail_name: self.guardrail_name,
            auth_token,
            base_url: resolve_text(parameters.base_url, &format!("{prefix}.base_url"), lookup)?,
            detector_id: resolve_text(
                parameters.detector_id,
                &format!("{prefix}.detector_id"),
                lookup,
            )?,
            is_detector_server: parameters.is_detector_server.unwrap_or(true),
            detector_params: optional.detector_params.unwrap_or_default(),
            extra_headers: optional.extra_headers.unwrap_or_default(),
            score_threshold: optional.score_threshold,
            block_on_detection: optional.block_on_detection.unwrap_or(true),
            ssl_verification: if parameters.verify_ssl.unwrap_or(true) {
                Verification::Enabled
            } else {
                Verification::Disabled
            },
        })
    }
}

fn resolve_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    value
        .map(|text| resolve_string(text, path, lookup))
        .transpose()
}
