use std::collections::BTreeMap;

use betterlite_config::types::LiteLlmParams;
use betterlite_core::api::bfl::generation_call::params::{CallParams, ParamsError};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Job {
    pub(super) model: String,
    pub(super) prompt: Option<String>,
    pub(super) image: ImageSpec,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) litellm_params: IndexMap<String, Json>,
    pub(super) params_kind: ParamsKind,
    pub(super) extra_headers: BTreeMap<String, String>,
    pub(super) timeout_millis: Option<u64>,
    pub(super) configured: Configured,
    pub(super) clock: ClockSettings,
    pub(super) submission: Reply,
    pub(super) polls: Vec<Reply>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ParamsKind {
    Dict,
    Typed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configured {
    pub(super) api_key: Option<String>,
    pub(super) api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClockSettings {
    pub(super) start_millis: u64,
    pub(super) tick_millis: u64,
    pub(super) sleep_advance_millis: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ImageSpec {
    Bytes { data: String },
    List { items: Vec<String> },
}

impl Job {
    // LiteLLM reads litellm_params either as a mapping or as GenericLiteLLMParams; both reach the same two fields
    pub(super) fn params(&self) -> Result<CallParams, ParamsError> {
        let dict = CallParams::from_dict(&self.litellm_params)?;
        match self.params_kind {
            ParamsKind::Dict => Ok(dict),
            ParamsKind::Typed => Ok(CallParams::from_typed(&LiteLlmParams {
                api_key: dict.api_key,
                api_base: dict.api_base,
                ssl_verify: None,
                model: None,
                api_version: None,
                timeout: None,
                stream_timeout: None,
                max_retries: None,
                rpm: None,
                tpm: None,
                weight: None,
            })),
        }
    }

    pub(super) fn extra_headers(&self) -> Vec<(String, String)> {
        self.extra_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}
