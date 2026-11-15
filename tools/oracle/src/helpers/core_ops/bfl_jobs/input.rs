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
    pub(super) prompt: String,
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

// LiteLLM takes litellm_params as a mapping or as GenericLiteLLMParams, and both reach the same two fields
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
    #[serde(rename = "start_millis")]
    pub(super) start: u64,
    #[serde(rename = "tick_millis")]
    pub(super) tick: u64,
    #[serde(rename = "sleep_advance_millis")]
    pub(super) sleep_advance: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
}

impl Job {
    pub(super) fn params(&self) -> Result<CallParams, ParamsError> {
        match self.params_kind {
            ParamsKind::Dict => CallParams::from_dict(&self.litellm_params),
            ParamsKind::Typed => Ok(CallParams::from_typed(&self.typed_params()?)),
        }
    }

    pub(super) fn extra_headers(&self) -> Vec<(String, String)> {
        self.extra_headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }

    fn typed_params(&self) -> Result<LiteLlmParams, ParamsError> {
        let params = CallParams::from_dict(&self.litellm_params)?;
        Ok(LiteLlmParams {
            ssl_verify: None,
            model: None,
            api_base: params.api_base,
            api_key: params.api_key,
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        })
    }
}
