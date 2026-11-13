use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::skills::config::AnthropicSkillsConfig;
use betterlite_anthropic::skills::config::BaseSkillsApiConfig as _;
use betterlite_anthropic::skills::wire::{CreateSkillRequest, Skill};
use betterlite_context::core_settings::CoreSettings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use http::header::HeaderMap;
use indexmap::IndexMap;

use crate::api::skills::get_delete::call::{LITELLM_PROXY_PROVIDER, resolved_timeout};
use crate::api::skills::get_delete::hosted::{HostedSkillsHandlers, hosted_handler};
use crate::api::skills::get_delete::service::api_base;

use super::call::{CreateSkillCall, SkillCreation};
use super::error::{CreateSkillError, CreateSkillFailureError, failure};
use super::handler::{CreateSkillHttpRequest, SkillsCreateHttpHandler};
use super::hosted::{HostedSkillsCreator, hosted_request};
use super::logging::{body_params, log_pre_call};
use super::request::assembled_request;

pub struct CreateSkillService<'deps, Http, Hosted> {
    pub http: &'deps Http,
    pub hosted: &'deps HostedSkillsHandlers<Hosted>,
    pub anthropic: &'deps AnthropicSettings,
    pub core: &'deps CoreSettings,
    pub redactor: &'deps Redactor,
}

struct PreparedCreate {
    provider: String,
    config: AnthropicSkillsConfig,
    url: String,
    headers: HeaderMap,
    body: CreateSkillRequest,
    timeout: Duration,
}

impl PreparedCreate {
    fn request(&self) -> CreateSkillHttpRequest<'_> {
        CreateSkillHttpRequest {
            url: self.url.clone(),
            headers: self.headers.clone(),
            timeout: self.timeout,
            body: &self.body,
            config: &self.config,
            custom_llm_provider: &self.provider,
        }
    }
}

impl<Http, Hosted> CreateSkillService<'_, Http, Hosted>
where
    Http: SkillsCreateHttpHandler,
    Hosted: HostedSkillsCreator,
{
    pub async fn create_skill(
        &self,
        request: SkillCreation<'_>,
    ) -> Result<Skill, CreateSkillFailureError> {
        let SkillCreation { call, logging } = request;
        match self.run(call, logging).await {
            Ok(skill) => Ok(skill),
            Err(error) => Err(failure(error, call, self.redactor)),
        }
    }

    async fn run(
        &self,
        call: &CreateSkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<Skill, CreateSkillError> {
        if call.params.provider() == LITELLM_PROXY_PROVIDER {
            let mut optional_params = IndexMap::new();
            optional_params.insert(
                "display_title".to_owned(),
                call.display_title
                    .map_or(Json::Null, |title| Json::Text(title.to_owned())),
            );
            log_pre_call(logging, call, LITELLM_PROXY_PROVIDER, optional_params)?;
            let handler = hosted_handler(self.hosted);
            let skill = handler.create_skill(hosted_request(call)?).await?;
            return Ok(skill);
        }
        let prepared = self.prepare(call, logging)?;
        let skill = self.http.create_skill(prepared.request()).await?;
        Ok(skill)
    }

    fn prepare(
        &self,
        call: &CreateSkillCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<PreparedCreate, CreateSkillError> {
        let provider = call.params.provider();
        let config = match provider {
            "anthropic" => AnthropicSkillsConfig {
                settings: self.anthropic.clone(),
            },
            _ => {
                return Err(CreateSkillError::Unsupported {
                    provider: provider.to_owned(),
                });
            }
        };
        let api_base = api_base(&call.params, self.anthropic);
        let mut headers = call.params.extra_headers.clone();
        config.validate_environment(&mut headers, call.params.api_key.as_ref(), Some(api_base))?;
        let assembled = assembled_request(call.display_title, call.files, call.extra_body)?;
        let body = config.transform_create_skill_request(&assembled);
        let url = config.get_complete_url(Some(api_base), "skills", None)?;
        log_pre_call(logging, call, provider, body_params(&body))?;
        let timeout = resolved_timeout(call.params.timeout, self.core)?;
        Ok(PreparedCreate {
            provider: provider.to_owned(),
            config,
            url,
            headers,
            body,
            timeout,
        })
    }
}
