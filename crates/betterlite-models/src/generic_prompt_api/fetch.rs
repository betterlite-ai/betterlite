use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

use crate::decisions::Json;
use crate::generic_prompt_api::error::PromptFetchError;
use crate::generic_prompt_api::manager::GenericPromptSettings;
use crate::generic_prompt_api::request::{
    PromptApiRequest, PromptApiResponse, PromptApiTransport, query_pairs,
};
use crate::prompt_contracts::{PromptManagementClient, PromptParams};
use crate::prompt_spec::PromptSpec;

pub const PROMPT_MANAGEMENT_PATH: &str = "/beta/litellm_prompt_management";

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct ApiPromptResponse {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub prompt_template: Vec<Json>,
    pub prompt_template_model: Option<String>,
    #[serde(rename = "prompt_template_optional_params")]
    pub optional_params: Option<PromptParams>,
}

impl ApiPromptResponse {
    // the prompt_id is the caller's, as in LiteLLM; the answer's own prompt_id is not read
    #[must_use]
    pub fn into_prompt_client(self, prompt_id: Option<&str>) -> PromptManagementClient {
        PromptManagementClient {
            prompt_id: prompt_id.map(str::to_owned),
            prompt_template: self.prompt_template,
            prompt_template_model: self.prompt_template_model,
            prompt_optional_params: self.optional_params,
            completed_messages: None,
        }
    }
}

#[derive(Debug)]
pub struct PromptApi<Transport> {
    pub settings: GenericPromptSettings,
    pub transport: Transport,
}

impl<Transport> PromptApi<Transport>
where
    Transport: PromptApiTransport,
{
    #[must_use]
    pub fn get_headers(&self) -> Vec<(&'static str, SecretString)> {
        let mut headers = vec![
            ("Content-Type", SecretString::from("application/json")),
            ("Accept", SecretString::from("application/json")),
        ];
        if let Some(api_key) = &self.settings.api_key {
            let bearer = format!("Bearer {}", api_key.expose_secret());
            headers.push(("Authorization", SecretString::from(bearer)));
        }
        headers
    }

    pub fn fetch_prompt_from_api(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
    ) -> Result<ApiPromptResponse, PromptFetchError<Transport::Error>> {
        let request = self.build_request(
            prompt_id,
            prompt_spec,
            self.settings.provider_specific_query_params.as_ref(),
        )?;
        let outcome = self.transport.get(&request);
        read_response(prompt_id, outcome)
    }

    pub async fn async_fetch_prompt_from_api(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
    ) -> Result<ApiPromptResponse, PromptFetchError<Transport::Error>> {
        let spec_params = prompt_spec
            .and_then(|spec| spec.litellm_params.provider_specific_query_params.as_ref());
        let request = self.build_request(prompt_id, prompt_spec, spec_params)?;
        let outcome = self.transport.get_async(&request).await;
        read_response(prompt_id, outcome)
    }

    fn build_request(
        &self,
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
        provider_specific: Option<&IndexMap<String, Json>>,
    ) -> Result<PromptApiRequest, PromptFetchError<Transport::Error>> {
        if prompt_id.is_none() && prompt_spec.is_none() {
            return Err(PromptFetchError::MissingPromptSource);
        }
        Ok(PromptApiRequest {
            url: format!("{}{PROMPT_MANAGEMENT_PATH}", self.settings.api_base),
            query: query_pairs(prompt_id, provider_specific)?,
            headers: self.get_headers(),
            timeout: self.settings.timeout,
        })
    }
}

fn read_response<TransportError>(
    prompt_id: Option<&str>,
    outcome: Result<PromptApiResponse, TransportError>,
) -> Result<ApiPromptResponse, PromptFetchError<TransportError>> {
    let prompt_name = prompt_id.unwrap_or("None").to_owned();
    let response = outcome.map_err(|source| PromptFetchError::Transport {
        prompt_id: prompt_name.clone(),
        source,
    })?;
    if !(200..300).contains(&response.status) {
        return Err(PromptFetchError::Status {
            prompt_id: prompt_name,
            status: response.status,
        });
    }
    serde_json::from_slice(&response.body).map_err(|source| PromptFetchError::Parse {
        prompt_id: prompt_name,
        source,
    })
}
