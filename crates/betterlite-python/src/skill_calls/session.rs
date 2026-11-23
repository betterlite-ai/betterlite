use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex};

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::skills::wire::{DeleteSkillResponse, ListSkillsResponse, Skill};
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::anthropic_credentials::Settings as CredentialSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_core::api::skills::create::call::{CreateSkillCall, SkillCreation};
use betterlite_core::api::skills::create::error::{CreateSkillError, failure as create_failure};
use betterlite_core::api::skills::create::service::CreateSkillService;
use betterlite_core::api::skills::get_delete::call::{
    LITELLM_PROXY_PROVIDER, SkillCall, SkillRequest,
};
use betterlite_core::api::skills::get_delete::error::{
    SkillAction, SkillsCallError, SkillsFailureError, failure as skill_failure,
};
use betterlite_core::api::skills::get_delete::hosted::HostedSkillsHandlers;
use betterlite_core::api::skills::get_delete::service::SkillsService;
use betterlite_core::api::skills::list::call::{ListSkillsCall, SkillListing};
use betterlite_core::api::skills::list::error::{ListSkillsError, failure as list_failure};
use betterlite_core::api::skills::list::service::ListSkillsService;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::skill_calls::convert;
use crate::skill_calls::error::{SessionError, SkillFailure, no_database_failure};
use crate::skill_calls::hosted::{NoDatabaseSkills, handlers};
use crate::skill_calls::transport::{CreateTransport, HttpSkills};

mod logging;

// The one answer a skill operation produces; untagged, so Python sees the provider's own JSON object.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum SkillOutcome {
    Skill(Skill),
    Deleted(DeleteSkillResponse),
    Listed(ListSkillsResponse),
}

// The process settings a skill session reads: no file config, so the process environment is the whole input.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SessionWire {
    environment: Option<BTreeMap<String, String>>,
}

// A settings error the session cannot start with; Python sees the message, never the source type.
fn setup<E: Display>(error: E) -> SessionError {
    SessionError::Setup {
        message: error.to_string(),
    }
}

// Everything one Python native session owns: the clients, the resolved credentials and the services' clock settings.
pub(super) struct SkillCore {
    http: HttpSkills,
    client: ProxiedClient,
    hosted: HostedSkillsHandlers<NoDatabaseSkills>,
    anthropic: AnthropicSettings,
    core: CoreSettings,
    redactor: Redactor,
}

impl SkillCore {
    pub(super) fn build(settings: &str) -> Result<Self, SessionError> {
        let wire: SessionWire =
            serde_json::from_str(settings).map_err(|source| SessionError::Json { source })?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment).map_err(setup)?;
        let client = ProxiedClient::outbound(&Arc::new(policy)).map_err(setup)?;
        let timeout = RequestTimeout::resolve(&environment).map_err(setup)?;
        let credentials = CredentialSettings::resolve_environment(&environment).map_err(setup)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(setup)?;
        Ok(Self {
            http: HttpSkills {
                client: client.clone(),
            },
            client,
            hosted: handlers(),
            anthropic: AnthropicSettings {
                api_key: credentials.api_key,
                auth_token: credentials.auth_token,
                api_base: credentials.api_base,
                use_bearer_for_custom_base: credentials.use_bearer_for_custom_base,
                anthropic_version: AnthropicSettings::default().anthropic_version,
            },
            core: CoreSettings {
                request_timeout_seconds: timeout.seconds,
                request_timeout_explicit: timeout.explicitly_set,
                litellm_request_timeout: None,
                model_timeout_overrides: Vec::new(),
            },
            redactor,
        })
    }

    pub(super) async fn skill(
        &self,
        action: SkillAction,
        arguments: IndexMap<String, Json>,
    ) -> Result<SkillOutcome, SkillFailure> {
        let arguments = convert::skill_arguments(arguments)?;
        let call = SkillCall {
            skill_id: &arguments.skill_id,
            params: arguments.params,
            kwargs: &arguments.kwargs,
            litellm_call_id: arguments.litellm_call_id.as_deref(),
        };
        let call_type = match action {
            SkillAction::Get => "get_skill",
            SkillAction::Delete => "delete_skill",
        };
        let mut logging = self.prepare(call_type, &call, &arguments.kwargs)?;
        let service = self.skills_service();
        let request = SkillRequest {
            call: &call,
            logging: &mut logging,
        };
        let answer: Result<SkillOutcome, SkillsFailureError> = match action {
            SkillAction::Get => service.get_skill(request).await.map(SkillOutcome::Skill),
            SkillAction::Delete => service
                .delete_skill(request)
                .await
                .map(SkillOutcome::Deleted),
        };
        answer.map_err(|failure| SkillFailure {
            class: failure.class,
        })
    }

    pub(super) async fn create(
        &self,
        arguments: IndexMap<String, Json>,
        blobs: Vec<Vec<u8>>,
    ) -> Result<SkillOutcome, SkillFailure> {
        let arguments = convert::create_arguments(arguments)?;
        let call = CreateSkillCall {
            params: arguments.params,
            display_title: arguments.display_title.as_deref(),
            files: arguments.files.as_deref(),
            extra_body: arguments.extra_body.as_ref(),
            kwargs: &arguments.kwargs,
            litellm_call_id: arguments.litellm_call_id.as_deref(),
        };
        if call.params.provider() == LITELLM_PROXY_PROVIDER {
            return Err(no_database_failure(&self.redactor));
        }
        let mut logging = logging::state(
            &self.redactor,
            "create_skill",
            &arguments.kwargs,
            arguments.litellm_call_id.as_deref(),
        )
        .map_err(|error| SkillFailure {
            class: create_failure(CreateSkillError::from(error), &call, &self.redactor).class,
        })?;
        self.create_skill(&call, &mut logging, blobs).await
    }

    // The create call itself: the transport holding this call's blobs, then the service that assembles the body.
    async fn create_skill(
        &self,
        call: &CreateSkillCall<'_>,
        logging: &mut LoggingState,
        blobs: Vec<Vec<u8>>,
    ) -> Result<SkillOutcome, SkillFailure> {
        let transport = CreateTransport {
            client: self.client.clone(),
            blobs: Mutex::new(blobs),
        };
        let service = CreateSkillService {
            http: &transport,
            hosted: &self.hosted,
            anthropic: &self.anthropic,
            core: &self.core,
            redactor: &self.redactor,
        };
        service
            .create_skill(SkillCreation { call, logging })
            .await
            .map(SkillOutcome::Skill)
            .map_err(|failure| SkillFailure {
                class: failure.class,
            })
    }

    pub(super) async fn list(
        &self,
        arguments: IndexMap<String, Json>,
    ) -> Result<SkillOutcome, SkillFailure> {
        let arguments = convert::list_arguments(arguments)?;
        let call = ListSkillsCall {
            params: arguments.params,
            limit: arguments.limit,
            page: arguments.page.as_deref(),
            source: arguments.source.as_deref(),
            kwargs: &arguments.kwargs,
            litellm_call_id: arguments.litellm_call_id.as_deref(),
        };
        if call.params.provider() == LITELLM_PROXY_PROVIDER {
            return Err(no_database_failure(&self.redactor));
        }
        let mut logging = logging::state(
            &self.redactor,
            "list_skills",
            &arguments.kwargs,
            arguments.litellm_call_id.as_deref(),
        )
        .map_err(|error| SkillFailure {
            class: list_failure(ListSkillsError::from(error), &call, &self.redactor).class,
        })?;
        self.list_skills(&call, &mut logging).await
    }

    // The list call: the shared read side, then the service that builds the query and pages the answer.
    async fn list_skills(
        &self,
        call: &ListSkillsCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<SkillOutcome, SkillFailure> {
        let service = ListSkillsService {
            http: &self.http,
            hosted: &self.hosted,
            anthropic: &self.anthropic,
            core: &self.core,
            redactor: &self.redactor,
        };
        service
            .list_skills(SkillListing { call, logging })
            .await
            .map(SkillOutcome::Listed)
            .map_err(|failure| SkillFailure {
                class: failure.class,
            })
    }

    // Both skill actions share the hosted check and the pre-call logging the services write into.
    fn prepare(
        &self,
        call_type: &'static str,
        call: &SkillCall<'_>,
        kwargs: &IndexMap<String, Json>,
    ) -> Result<LoggingState, SkillFailure> {
        if call.params.provider() == LITELLM_PROXY_PROVIDER {
            return Err(no_database_failure(&self.redactor));
        }
        logging::state(&self.redactor, call_type, kwargs, call.litellm_call_id).map_err(|error| {
            SkillFailure {
                class: skill_failure(SkillsCallError::from(error), call, &self.redactor).class,
            }
        })
    }

    fn skills_service(&self) -> SkillsService<'_, HttpSkills, NoDatabaseSkills> {
        SkillsService {
            http: &self.http,
            hosted: &self.hosted,
            anthropic: &self.anthropic,
            core: &self.core,
            redactor: &self.redactor,
        }
    }
}
