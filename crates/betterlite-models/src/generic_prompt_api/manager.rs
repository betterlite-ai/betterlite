use std::time::Duration;

use indexmap::IndexMap;
use secrecy::SecretString;

use crate::decisions::Json;
use crate::generic_prompt_api::error::PromptCompileError;
use crate::generic_prompt_api::fetch::PromptApi;
use crate::generic_prompt_api::request::PromptApiTransport;
use crate::generic_prompt_api::variables::apply_variables;
use crate::generic_prompt_api_cache::PromptCache;
use crate::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptIgnoreFlags, PromptManagementClient,
    PromptParams,
};
use crate::prompt_spec::PromptSpec;

#[derive(Clone, Debug)]
pub struct GenericPromptSettings {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub timeout: Duration,
    pub prompt_id: Option<String>,
    pub provider_specific_query_params: Option<IndexMap<String, Json>>,
    pub ignore_flags: PromptIgnoreFlags,
}

#[derive(Debug)]
pub struct GenericPromptManager<Transport> {
    pub api: PromptApi<Transport>,
    pub prompt_cache: PromptCache,
}

impl<Transport> GenericPromptManager<Transport>
where
    Transport: PromptApiTransport,
{
    pub const INTEGRATION_NAME: &'static str = "generic_prompt";

    pub fn new(mut settings: GenericPromptSettings, transport: Transport) -> Self {
        let trimmed_length = settings.api_base.trim_end_matches('/').len();
        settings.api_base.truncate(trimmed_length);
        Self {
            api: PromptApi {
                settings,
                transport,
            },
            prompt_cache: PromptCache::default(),
        }
    }

    #[must_use]
    pub fn should_run_prompt_management(
        prompt_id: Option<&str>,
        prompt_spec: Option<&PromptSpec>,
    ) -> bool {
        prompt_id.is_some()
            || prompt_spec
                .is_some_and(|spec| spec.litellm_params.provider_specific_query_params.is_some())
    }

    pub fn compile_prompt_helper(
        &self,
        request: &PromptCompileRequest<'_>,
    ) -> Result<PromptManagementClient, PromptCompileError<Transport::Error>> {
        if let Some(cached) = self.prompt_cache.common_caching_logic(
            request.prompt_id,
            request.prompt_label,
            request.prompt_version,
            request.prompt_variables,
        )? {
            return Ok(cached);
        }
        let api_response = self
            .api
            .fetch_prompt_from_api(request.prompt_id, request.prompt_spec)
            .map_err(|source| PromptCompileError::Fetch {
                prompt_id: request.prompt_id.unwrap_or("None").to_owned(),
                source,
            })?;
        let cache_key = PromptCache::get_cache_key(
            request.prompt_id,
            request.prompt_label,
            request.prompt_version,
        );
        let prompt_client = api_response.into_prompt_client(request.prompt_id);
        self.cache_and_apply(cache_key, request.prompt_variables, prompt_client)
    }

    pub async fn async_compile_prompt_helper(
        &self,
        request: &PromptAsyncCompileRequest<'_>,
    ) -> Result<PromptManagementClient, PromptCompileError<Transport::Error>> {
        if let Some(cached) = self.prompt_cache.common_caching_logic(
            request.prompt_id,
            request.prompt_label,
            request.prompt_version,
            request.prompt_variables,
        )? {
            return Ok(cached);
        }
        let api_response = self
            .api
            .async_fetch_prompt_from_api(request.prompt_id, request.prompt_spec)
            .await
            .map_err(|source| PromptCompileError::Fetch {
                prompt_id: request.prompt_id.unwrap_or("None").to_owned(),
                source,
            })?;
        let cache_key = PromptCache::get_cache_key(
            request.prompt_id,
            request.prompt_label,
            request.prompt_version,
        );
        let prompt_client = api_response.into_prompt_client(request.prompt_id);
        self.cache_and_apply(cache_key, request.prompt_variables, prompt_client)
    }

    fn cache_and_apply(
        &self,
        cache_key: String,
        prompt_variables: Option<&PromptParams>,
        prompt_client: PromptManagementClient,
    ) -> Result<PromptManagementClient, PromptCompileError<Transport::Error>> {
        self.prompt_cache.insert(cache_key, prompt_client.clone());
        match prompt_variables {
            Some(variables) if !variables.is_empty() => {
                Ok(apply_variables(&prompt_client, variables)?)
            }
            _ => Ok(prompt_client),
        }
    }
}
