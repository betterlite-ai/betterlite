#![cfg(test)]

use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::loader::{ConfigError, parse_view};

mod tests;

// the generation values LiteLLM constructs a deployment's config with; every one starts unset
#[derive(Debug, Default, Deserialize, PartialEq)]
pub(crate) struct GenerationDefaults {
    pub(crate) temperature: Option<f64>,
    pub(crate) max_output_tokens: Option<i64>,
    pub(crate) top_p: Option<f64>,
    pub(crate) top_k: Option<i64>,
    pub(crate) response_mime_type: Option<String>,
    pub(crate) candidate_count: Option<i64>,
    pub(crate) stop_sequences: Option<Vec<String>>,
    pub(crate) frequency_penalty: Option<f64>,
    pub(crate) presence_penalty: Option<f64>,
    pub(crate) seed: Option<i64>,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Settings {
    pub(crate) deployments: Vec<GenerationDefaults>,
    pub(crate) safety_settings: Option<Json>,
}

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: GenerationDefaults,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    vertex_ai_safety_settings: Option<Json>,
}

// The section registry in sections.rs has no entry for this key yet, so the loader's own view parses it here until it does
pub(crate) fn load(source: &str) -> Result<Settings, ConfigError> {
    let (raw, _unknown) = parse_view::<Raw>(source)?;
    Ok(Settings {
        deployments: raw
            .model_list
            .into_iter()
            .map(|deployment| deployment.litellm_params)
            .collect(),
        safety_settings: raw.litellm_settings.vertex_ai_safety_settings,
    })
}
