use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::skills::wire::{DeleteSkillResponse, ListSkillsResponse, Skill};
use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::anthropic_credentials::Settings as CredentialSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_core::api::skills::get_delete::call::{
    LITELLM_PROXY_PROVIDER, SkillCall, SkillRequest,
};
use betterlite_core::api::skills::get_delete::error::{
    SkillAction, SkillsCallError, SkillsFailureError, failure as skill_failure,
};
use betterlite_core::api::skills::get_delete::hosted::HostedSkillsHandlers;
use betterlite_core::api::skills::get_delete::service::SkillsService;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::Utc;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::skill_calls::convert;
use crate::skill_calls::error::{SessionError, SkillFailure, no_database_failure};
use crate::skill_calls::hosted::NoDatabaseSkills;
use crate::skill_calls::transport::HttpSkills;

mod create;
mod list;

// The one answer a skill operation produces; untagged, so Python sees the provider's own JSON object.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum SkillOutcome {
    Skill(Skill),
    Deleted(DeleteSkillResponse),
    Listed(ListSkillsResponse),
}

pub(super) type Answer = Result<SkillOutcome, SkillFailure>;
// One call's parsed arguments and the file bytes their $file markers name.
pub(super) type Call = (IndexMap<String, Json>, Vec<Vec<u8>>);

// The operation one Python call asks for, named after the session method that took it.
#[derive(Clone, Copy)]
pub(super) enum Mode {
    Create,
    List,
    Get,
    Delete,
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

// The pre-call update every service runs needs a logging state; the Python package registers no callbacks.
pub(super) fn call_logging(
    redactor: &Redactor,
    call_type: &str,
    kwargs: &IndexMap<String, Json>,
    litellm_call_id: Option<&str>,
) -> Result<LoggingState, LoggingStateError> {
    let init = LoggingInit {
        model: String::new(),
        call_type: call_type.to_owned(),
        start_time: Utc::now().fixed_offset(),
        litellm_call_id: litellm_call_id.map_or_else(|| Uuid::new_v4().to_string(), str::to_owned),
        function_id: String::new(),
        messages: None,
        stream: None,
        litellm_trace_id: None,
        kwargs: kwargs.clone(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || Uuid::new_v4().to_string(),
        Duration::default,
    )
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
            hosted: HostedSkillsHandlers {
                create: Box::new(|| Arc::new(NoDatabaseSkills)),
                cached: Mutex::new(None),
            },
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

    pub(super) async fn skill(&self, action: SkillAction, args: IndexMap<String, Json>) -> Answer {
        let arguments = convert::skill::arguments(args)?;
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
        let service = SkillsService {
            http: &self.http,
            hosted: &self.hosted,
            anthropic: &self.anthropic,
            core: &self.core,
            redactor: &self.redactor,
        };
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

    // One Python call dispatched to the operation its session method names.
    pub(super) async fn execute(&self, mode: Mode, call: Call) -> Answer {
        let (args, blobs) = call;
        match mode {
            Mode::Create => create::skill(self, args, blobs).await,
            Mode::List => list::skill(self, args).await,
            Mode::Get => self.skill(SkillAction::Get, args).await,
            Mode::Delete => self.skill(SkillAction::Delete, args).await,
        }
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
        call_logging(&self.redactor, call_type, kwargs, call.litellm_call_id).map_err(|error| {
            SkillFailure {
                class: skill_failure(SkillsCallError::from(error), call, &self.redactor).class,
            }
        })
    }
}
