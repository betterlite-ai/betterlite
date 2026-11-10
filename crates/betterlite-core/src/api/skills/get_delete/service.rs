use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::skills::config::{AnthropicSkillsConfig, BaseSkillsApiConfig};
use betterlite_anthropic::skills::error::SkillsError;
use betterlite_anthropic::skills::wire::{DeleteSkillResponse, Skill};
use betterlite_context::core_settings::CoreSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;
use http::header::HeaderMap;

use super::call::{LITELLM_PROXY_PROVIDER, SkillCall, SkillsCallParams, resolved_timeout};
use super::error::{SkillAction, SkillsCallError, SkillsFailureError, failure};
use super::handler::{SkillHttpRequest, SkillsHttpHandler};
use super::hosted::{
    HostedSkillRequest, HostedSkillsHandler, HostedSkillsHandlers, hosted_handler,
};
use super::logging::log_pre_call;
use super::principal::user_api_key_auth_from_kwargs;

pub struct SkillsService<'deps, Http, Hosted> {
    pub http: &'deps Http,
    pub hosted: &'deps HostedSkillsHandlers<Hosted>,
    pub anthropic: &'deps AnthropicSettings,
    pub core: &'deps CoreSettings,
    pub redactor: &'deps Redactor,
}

struct PreparedSkill {
    provider: String,
    config: AnthropicSkillsConfig,
    url: String,
    headers: HeaderMap,
    timeout: Duration,
}

impl<Http, Hosted> SkillsService<'_, Http, Hosted>
where
    Http: SkillsHttpHandler,
    Hosted: HostedSkillsHandler,
{
    pub async fn get_skill(
        &self,
        call: &SkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<Skill, SkillsFailureError> {
        match self.run_get(call, logging).await {
            Ok(skill) => Ok(skill),
            Err(error) => Err(failure(error, call.params.provider(), self.redactor)),
        }
    }

    pub async fn delete_skill(
        &self,
        call: &SkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<DeleteSkillResponse, SkillsFailureError> {
        match self.run_delete(call, logging).await {
            Ok(deleted) => Ok(deleted),
            Err(error) => Err(failure(error, call.params.provider(), self.redactor)),
        }
    }

    async fn run_get(
        &self,
        call: &SkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<Skill, SkillsCallError> {
        if call.params.provider() == LITELLM_PROXY_PROVIDER {
            return self.hosted_get(call, logging).await;
        }
        let prepared = self.prepare(SkillAction::Get, call, logging)?;
        let request = SkillHttpRequest {
            url: prepared.url,
            headers: prepared.headers,
            timeout: prepared.timeout,
            config: &prepared.config,
            custom_llm_provider: &prepared.provider,
        };
        Ok(self.http.get_skill(request).await?)
    }

    async fn run_delete(
        &self,
        call: &SkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<DeleteSkillResponse, SkillsCallError> {
        if call.params.provider() == LITELLM_PROXY_PROVIDER {
            return self.hosted_delete(call, logging).await;
        }
        let prepared = self.prepare(SkillAction::Delete, call, logging)?;
        let request = SkillHttpRequest {
            url: prepared.url,
            headers: prepared.headers,
            timeout: prepared.timeout,
            config: &prepared.config,
            custom_llm_provider: &prepared.provider,
        };
        Ok(self.http.delete_skill(request).await?)
    }

    async fn hosted_get(
        &self,
        call: &SkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<Skill, SkillsCallError> {
        log_pre_call(logging, call, LITELLM_PROXY_PROVIDER)?;
        let handler = hosted_handler(self.hosted);
        Ok(handler.get_skill(hosted_request(call)).await?)
    }

    async fn hosted_delete(
        &self,
        call: &SkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<DeleteSkillResponse, SkillsCallError> {
        log_pre_call(logging, call, LITELLM_PROXY_PROVIDER)?;
        let handler = hosted_handler(self.hosted);
        Ok(handler.delete_skill(hosted_request(call)).await?)
    }

    fn prepare(
        &self,
        action: SkillAction,
        call: &SkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<PreparedSkill, SkillsCallError> {
        let provider = call.params.provider();
        let config = skills_api_config(provider, self.anthropic).ok_or_else(|| {
            SkillsCallError::Unsupported {
                action,
                provider: provider.to_owned(),
            }
        })?;
        let api_base = api_base(&call.params, self.anthropic);
        let mut headers = call.params.extra_headers.clone();
        config.validate_environment(&mut headers, call.params.api_key.as_ref(), Some(api_base))?;
        let url = request_url(&config, action, call.skill_id, api_base)?;
        log_pre_call(logging, call, provider)?;
        let timeout = resolved_timeout(call.params.timeout, self.core)?;
        Ok(PreparedSkill {
            provider: provider.to_owned(),
            config,
            url,
            headers,
            timeout,
        })
    }
}

fn hosted_request<'call>(call: &'call SkillCall<'_>) -> HostedSkillRequest<'call> {
    HostedSkillRequest {
        skill_id: call.skill_id,
        user_api_key_auth: user_api_key_auth_from_kwargs(call.kwargs),
        litellm_call_id: call.litellm_call_id,
    }
}

// ProviderConfigManager.get_provider_skills_api_config: Anthropic is the only provider with a Skills config
fn skills_api_config(
    provider: &str,
    settings: &AnthropicSettings,
) -> Option<AnthropicSkillsConfig> {
    match provider {
        "anthropic" => Some(AnthropicSkillsConfig {
            settings: settings.clone(),
        }),
        _ => None,
    }
}

// AnthropicModelInfo.get_api_base(litellm_params.api_base) with B567's resolved settings as the environment
fn api_base<'call>(
    params: &'call SkillsCallParams,
    settings: &'call AnthropicSettings,
) -> &'call str {
    match params.api_base.as_deref() {
        Some(base) if !base.is_empty() => base,
        Some(_) | None => &settings.api_base,
    }
}

fn request_url(
    config: &dyn BaseSkillsApiConfig,
    action: SkillAction,
    skill_id: &str,
    api_base: &str,
) -> Result<String, SkillsError> {
    match action {
        SkillAction::Get => config.transform_get_skill_request(skill_id, Some(api_base)),
        SkillAction::Delete => config.transform_delete_skill_request(skill_id, Some(api_base)),
    }
}
