use std::future::{Future, ready};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};
use tokio::time::timeout;

use crate::fetch_policy::response::FetchResponse;
use crate::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};

// litellm.constants.HF_CONFIG_FETCH_TIMEOUT_SECONDS, shortened for the name-length rule
pub const HF_CONFIG_TIMEOUT_SECONDS: f64 = 10.0;
// B173's buffered-body limit, so a runaway config.json answer fails the fetch instead of growing memory.
pub const HF_CONFIG_BODY_LIMIT: usize = 0x0100_0000;
pub const HF_CONFIG_BASE_URL: &str = "https://huggingface.co";
const CONFIG_PATH: &str = "raw/main/config.json";

#[derive(Debug, thiserror::Error)]
pub enum HfConfigError {
    #[error("the Hugging Face config has no integer max_position_embeddings")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Default, Deserialize)]
struct HfConfigDocument {
    #[serde(default)]
    max_position_embeddings: Option<i64>,
}

pub fn decode_hf_model_config(bytes: &[u8]) -> Result<Option<i64>, HfConfigError> {
    match serde_json::from_slice::<HfConfigDocument>(bytes) {
        Ok(document) => Ok(document.max_position_embeddings),
        Err(source) => Err(HfConfigError::Json(source)),
    }
}

// ModelInfoBase's Hugging Face shape: every other capability stays unknown.
#[derive(Debug, Serialize)]
struct HuggingfaceModelInfo<'model> {
    key: &'model str,
    max_tokens: Option<i64>,
    max_input_tokens: Option<i64>,
    max_output_tokens: Option<i64>,
    input_cost_per_token: u8,
    output_cost_per_token: u8,
    litellm_provider: &'static str,
    mode: &'static str,
    supports_system_messages: Option<bool>,
    supports_response_schema: Option<bool>,
    supports_function_calling: Option<bool>,
    supports_tool_choice: Option<bool>,
    supports_assistant_prefill: Option<bool>,
    supports_prompt_caching: Option<bool>,
    #[serde(rename = "supports_prompt_cache_breakpoint")]
    prompt_cache_breakpoint: Option<bool>,
    #[serde(rename = "supports_thinking_cache_preservation")]
    thinking_cache_preservation: Option<bool>,
    supports_computer_use: Option<bool>,
    supports_pdf_input: Option<bool>,
}

pub(super) fn huggingface_model_info(
    model: &str,
    max_tokens: Option<i64>,
) -> Result<Box<RawValue>, serde_json::Error> {
    to_raw_value(&HuggingfaceModelInfo {
        key: model,
        max_tokens,
        max_input_tokens: None,
        max_output_tokens: None,
        input_cost_per_token: 0,
        output_cost_per_token: 0,
        litellm_provider: "huggingface",
        mode: "chat",
        supports_system_messages: None,
        supports_response_schema: None,
        supports_function_calling: None,
        supports_tool_choice: None,
        supports_assistant_prefill: None,
        supports_prompt_caching: None,
        prompt_cache_breakpoint: None,
        thinking_cache_preservation: None,
        supports_computer_use: None,
        supports_pdf_input: None,
    })
}

#[derive(Debug)]
pub struct HfConfigClient<Client> {
    pub client: Client,
    pub base_url: String,
}

impl<Client: ModelInfoClient> HfConfigClient<Client> {
    pub async fn max_position_embeddings(&self, model: &str) -> Option<i64> {
        let timeout_duration = Duration::try_from_secs_f64(HF_CONFIG_TIMEOUT_SECONDS).ok()?;
        let request = ModelInfoRequest {
            url: format!(
                "{}/{model}/{CONFIG_PATH}",
                self.base_url.trim_end_matches('/')
            ),
            headers: Vec::new(),
            timeout: timeout_duration,
        };
        let response = timeout(timeout_duration, self.client.get(&request))
            .await
            .ok()?
            .ok()?;
        if !(200..300).contains(&response.status) || response.body.len() > HF_CONFIG_BODY_LIMIT {
            return None;
        }
        decode_hf_model_config(&response.body).ok().flatten()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UnreachableConfigTransport;

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("no Hugging Face config transport is configured")]
pub struct UnreachableConfig;

impl ModelInfoClient for UnreachableConfigTransport {
    type Error = UnreachableConfig;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, UnreachableConfig>> + Send {
        ready(Err(UnreachableConfig))
    }
}

impl HfConfigClient<UnreachableConfigTransport> {
    #[must_use]
    pub fn unreachable() -> Self {
        Self {
            client: UnreachableConfigTransport,
            base_url: HF_CONFIG_BASE_URL.to_owned(),
        }
    }
}

mod tests;
