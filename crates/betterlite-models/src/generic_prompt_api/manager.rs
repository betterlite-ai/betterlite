use std::time::Duration;

use indexmap::IndexMap;
use secrecy::SecretString;

use crate::decisions::Json;
use crate::generic_prompt_api::error::{PromptCompileError, PromptFetchError};
use crate::generic_prompt_api::fetch::{ApiPromptResponse, PromptApi};
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
    pub provider_specific_query_params: IndexMap<String, Json>,
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
        let cached = self.prompt_cache.common_caching_logic(
            request.prompt_id,
            request.prompt_label,
            request.prompt_version,
            request.prompt_variables,
        )?;
        cached.map_or_else(
            || {
                self.cache_fetched(
                    request.prompt_id,
                    PromptCache::get_cache_key(
                        request.prompt_id,
                        request.prompt_label,
                        request.prompt_version,
                    ),
                    request.prompt_variables,
                    self.api
                        .fetch_prompt_from_api(request.prompt_id, request.prompt_spec),
                )
            },
            Ok,
        )
    }

    pub async fn async_compile_prompt_helper(
        &self,
        request: &PromptAsyncCompileRequest<'_>,
    ) -> Result<PromptManagementClient, PromptCompileError<Transport::Error>> {
        let cache_key = PromptCache::get_cache_key(
            request.prompt_id,
            request.prompt_label,
            request.prompt_version,
        );
        if let Some(cached) = self.prompt_cache.common_caching_logic(
            request.prompt_id,
            request.prompt_label,
            request.prompt_version,
            request.prompt_variables,
        )? {
            return Ok(cached);
        }
        let fetched = self
            .api
            .async_fetch_prompt_from_api(request.prompt_id, request.prompt_spec)
            .await;
        self.cache_fetched(
            request.prompt_id,
            cache_key,
            request.prompt_variables,
            fetched,
        )
    }

    fn cache_fetched(
        &self,
        prompt_id: Option<&str>,
        cache_key: String,
        prompt_variables: Option<&PromptParams>,
        fetched: Result<ApiPromptResponse, PromptFetchError<Transport::Error>>,
    ) -> Result<PromptManagementClient, PromptCompileError<Transport::Error>> {
        let api_response = fetched.map_err(|source| PromptCompileError::Fetch {
            prompt_id: prompt_id.unwrap_or("None").to_owned(),
            source,
        })?;
        // the prompt_id is the caller's, as in LiteLLM; the answer's own prompt_id is not read
        let prompt_client = PromptManagementClient {
            prompt_id: prompt_id.map(str::to_owned),
            prompt_template: api_response.prompt_template,
            prompt_template_model: api_response.prompt_template_model,
            prompt_optional_params: api_response.optional_params,
            completed_messages: None,
        };
        self.prompt_cache.insert(cache_key, prompt_client.clone());
        match prompt_variables {
            Some(variables) if !variables.is_empty() => {
                Ok(apply_variables(&prompt_client, variables)?)
            }
            _ => Ok(prompt_client),
        }
    }
}
