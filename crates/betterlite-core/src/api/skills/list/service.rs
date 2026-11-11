use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::skills::config::AnthropicSkillsConfig;
use betterlite_anthropic::skills::config::BaseSkillsApiConfig as _;
use betterlite_anthropic::skills::wire::ListSkillsResponse;
use betterlite_context::core_settings::CoreSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;
use http::header::HeaderMap;

use crate::api::skills::get_delete::call::{LITELLM_PROXY_PROVIDER, resolved_timeout};
use crate::api::skills::get_delete::hosted::{HostedSkillsHandlers, hosted_handler};
use crate::api::skills::get_delete::principal::user_api_key_auth_from_kwargs;
use crate::api::skills::get_delete::service::api_base;

use super::call::ListSkillsCall;
use super::call::SkillListing;
use super::error::{ListSkillsError, ListSkillsFailureError, failure};
use super::handler::{ListSkillsHttpRequest, SkillsListHttpHandler};
use super::hosted::{DEFAULT_HOSTED_LIMIT, HostedListSkillsRequest, HostedSkillsLister};
use super::logging::{log_hosted_pre_call, log_pre_call};
use super::params::list_params;

pub struct ListSkillsService<'deps, Http, Hosted> {
    pub http: &'deps Http,
    pub hosted: &'deps HostedSkillsHandlers<Hosted>,
    pub anthropic: &'deps AnthropicSettings,
    pub core: &'deps CoreSettings,
    pub redactor: &'deps Redactor,
}

struct PreparedList {
    provider: String,
    config: AnthropicSkillsConfig,
    url: String,
    query_params: Vec<(String, String)>,
    headers: HeaderMap,
    timeout: Duration,
}

impl PreparedList {
    fn request(&self) -> ListSkillsHttpRequest<'_> {
        ListSkillsHttpRequest {
            url: self.url.clone(),
            query_params: self.query_params.clone(),
            headers: self.headers.clone(),
            timeout: self.timeout,
            config: &self.config,
            custom_llm_provider: &self.provider,
        }
    }
}

impl<Http, Hosted> ListSkillsService<'_, Http, Hosted>
where
    Http: SkillsListHttpHandler,
    Hosted: HostedSkillsLister,
{
    pub async fn list_skills(
        &self,
        request: SkillListing<'_>,
    ) -> Result<ListSkillsResponse, ListSkillsFailureError> {
        let SkillListing { call, logging } = request;
        match self.run(call, logging).await {
            Ok(response) => Ok(response),
            Err(error) => Err(failure(error, call, self.redactor)),
        }
    }

    async fn run(
        &self,
        call: &ListSkillsCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<ListSkillsResponse, ListSkillsError> {
        if call.params.provider() == LITELLM_PROXY_PROVIDER {
            // LiteLLM passes `limit or 20` and a fixed offset of 0, and never forwards page, source or extra_query
            let limit = call
                .limit
                .filter(|limit| *limit != 0)
                .unwrap_or(DEFAULT_HOSTED_LIMIT);
            let offset = 0;
            log_hosted_pre_call(logging, call, limit, offset)?;
            let handler = hosted_handler(self.hosted);
            let response = handler
                .list_skills(HostedListSkillsRequest {
                    limit,
                    offset,
                    user_api_key_auth: user_api_key_auth_from_kwargs(call.kwargs),
                    litellm_call_id: call.litellm_call_id,
                })
                .await?;
            return Ok(response);
        }
        let prepared = self.prepare(call, logging)?;
        let response = self.http.list_skills(prepared.request()).await?;
        Ok(response)
    }

    fn prepare(
        &self,
        call: &ListSkillsCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<PreparedList, ListSkillsError> {
        let provider = call.params.provider();
        let config = match provider {
            "anthropic" => AnthropicSkillsConfig {
                settings: self.anthropic.clone(),
            },
            _ => {
                return Err(ListSkillsError::Unsupported {
                    provider: provider.to_owned(),
                });
            }
        };
        let params = list_params(call)?;
        let api_base = api_base(&call.params, self.anthropic);
        let mut headers = call.params.extra_headers.clone();
        config.validate_environment(&mut headers, call.params.api_key.as_ref(), Some(api_base))?;
        let (url, query_params) = config.transform_list_skills_request(&params, Some(api_base))?;
        log_pre_call(logging, call, provider, &query_params)?;
        let timeout = resolved_timeout(call.params.timeout, self.core)?;
        Ok(PreparedList {
            provider: provider.to_owned(),
            config,
            url,
            query_params,
            headers,
            timeout,
        })
    }
}
