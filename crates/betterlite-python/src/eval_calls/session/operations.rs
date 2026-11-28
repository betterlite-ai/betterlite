use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::startup::process_environment;
use betterlite_context::core_settings::CoreSettings;
use betterlite_core::api::eval_create_list::call::{
    CreateEvalCall, EvalCreation, EvalListing, EvalsCallParams, ListEvalsCall,
};
use betterlite_core::api::eval_create_list::handler::BASE_LLM_HTTP_HANDLER;
use betterlite_core::api::eval_create_list::service::EvalsCreateListService;
use betterlite_core::api::eval_get_update::call::{
    EvalRetrieval, EvalUpdate, GetEvalCall, UpdateEvalCall,
};
use betterlite_core::api::eval_get_update::service::EvalsGetUpdateService;
use betterlite_core::api::eval_lifecycle::{EvalLifecycleCall, EvalLifecycleDeps, EvalLifecycles};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::EvalCore;
use super::logging::{self, SILENT_SINK};
use crate::completion::setup::outbound_client;
use crate::eval_calls::configs::EvalConfigs;
use crate::eval_calls::convert::{SessionWire, parse};
use crate::eval_calls::error::{EvalOperationFailure, FailureGroup, SessionError};
use crate::eval_calls::payload::{CreatePayload, IdPayload, ListPayload, Params, UpdatePayload};

// Each arm's payload is large, so it stays on the heap and the enum holds one word plus its tag.
pub(super) enum EvalCall {
    Request(Box<RequestCall>),
    Delete(Box<IdPayload>),
    Cancel(Box<IdPayload>),
}

pub(super) enum RequestCall {
    Create(CreatePayload),
    List(ListPayload),
    Get(IdPayload),
    Update(UpdatePayload),
}

// Delete and cancel take the same id payload and differ in the transform and handler the service picks.
#[derive(Clone, Copy)]
pub(super) enum Action {
    Delete,
    Cancel,
}

impl RequestCall {
    const fn label(&self) -> (FailureGroup, &'static str, &Params) {
        match self {
            Self::Create(payload) => (FailureGroup::CreateList, "create_eval", &payload.params),
            Self::List(payload) => (FailureGroup::CreateList, "list_evals", &payload.params),
            Self::Get(payload) => (FailureGroup::GetUpdate, "get_eval", &payload.params),
            Self::Update(payload) => (FailureGroup::GetUpdate, "update_eval", &payload.params),
        }
    }
}

impl EvalCore {
    pub(super) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let client = outbound_client(&environment)?;
        let timeout = RequestTimeout::resolve(&environment)?;
        let request_timeout = Duration::try_from_secs_f64(timeout.seconds).map_err(|source| {
            SessionError::Duration {
                seconds: timeout.seconds,
                source,
            }
        })?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        let configs = EvalConfigs::new(wire.settings, &environment);
        Ok(Self {
            configs,
            client,
            core: CoreSettings {
                request_timeout_seconds: timeout.seconds,
                request_timeout_explicit: timeout.explicitly_set,
                litellm_request_timeout: None,
                model_timeout_overrides: Vec::new(),
            },
            redactor,
            request_timeout,
        })
    }

    pub(super) async fn execute(&self, call: EvalCall) -> Result<String, EvalOperationFailure> {
        match call {
            EvalCall::Request(request) => self.request(&request).await,
            EvalCall::Delete(payload) => self.lifecycle(&payload, Action::Delete).await,
            EvalCall::Cancel(payload) => self.lifecycle(&payload, Action::Cancel).await,
        }
    }

    // What every call does before its service runs: build the logging state and the environment its pre-call reads.
    fn started<'lib, 'call>(
        &'lib self,
        group: FailureGroup,
        call_type: &str,
        params: &'call Params,
    ) -> Result<(LoggingState, PrePostEnv<'lib>, EvalsCallParams<'call>), EvalOperationFailure>
    {
        let evals = params.call();
        let state = logging::call_logging(self, call_type, &params.kwargs).map_err(|source| {
            EvalOperationFailure::mapped(group.class(source, &evals, &self.redactor), &evals)
        })?;
        let env = logging::pre_post_env(self, &SILENT_SINK);
        Ok((state, env, evals))
    }

    // The four requests share one driver; each arm builds the call its handler takes and the service reads it with.
    async fn request(&self, call: &RequestCall) -> Result<String, EvalOperationFailure> {
        let (group, call_type, params) = call.label();
        #[rustfmt::skip]
        let (mut state, env, evals) = self.started(group, call_type, params)?;
        #[rustfmt::skip]
        let create_list = EvalsCreateListService { handler: &BASE_LLM_HTTP_HANDLER, configs: &self.configs, client: &self.client, core: &self.core, redactor: &self.redactor };
        #[rustfmt::skip]
        let get_update = EvalsGetUpdateService { handler: &BASE_LLM_HTTP_HANDLER, configs: &self.configs, client: &self.client, core: &self.core, redactor: &self.redactor };
        match call {
            RequestCall::Create(payload) => {
                #[rustfmt::skip]
                let request = CreateEvalCall { data_source_config: payload.data_source_config.clone(), testing_criteria: payload.testing_criteria.clone(), name: payload.name.as_deref(), metadata: payload.metadata.as_ref(), extra_body: &payload.extra_body, params: payload.params.call() };
                #[rustfmt::skip]
                let reply = create_list.create_eval(EvalCreation { call: &request, logging: &mut state, env: &env }).await;
                EvalOperationFailure::reply(reply, &evals)
            }
            RequestCall::List(payload) => {
                #[rustfmt::skip]
                let request = ListEvalsCall { limit: payload.limit, after: payload.after.as_deref(), before: payload.before.as_deref(), order: payload.order, order_by: payload.order_by, params: payload.params.call() };
                #[rustfmt::skip]
                let reply = create_list.list_evals(EvalListing { call: &request, logging: &mut state, env: &env }).await;
                EvalOperationFailure::reply(reply, &evals)
            }
            RequestCall::Get(payload) => {
                #[rustfmt::skip]
                let request = GetEvalCall { eval_id: &payload.eval_id, params: payload.params.call() };
                #[rustfmt::skip]
                let reply = get_update.get_eval(EvalRetrieval { call: &request, logging: &mut state, env: &env }).await;
                EvalOperationFailure::reply(reply, &evals)
            }
            RequestCall::Update(payload) => {
                #[rustfmt::skip]
                let request = UpdateEvalCall { eval_id: &payload.eval_id, name: payload.name.as_deref(), metadata: payload.metadata.as_ref(), extra_body: &payload.extra_body, params: payload.params.call() };
                #[rustfmt::skip]
                let reply = get_update.update_eval(EvalUpdate { call: &request, logging: &mut state, env: &env }).await;
                EvalOperationFailure::reply(reply, &evals)
            }
        }
    }

    // The lifecycle service is built from its dependencies and the logging exchange; both calls share that setup.
    async fn lifecycle(
        &self,
        payload: &IdPayload,
        action: Action,
    ) -> Result<String, EvalOperationFailure> {
        let call_type = match action {
            Action::Delete => "delete_eval",
            Action::Cancel => "cancel_eval",
        };
        #[rustfmt::skip]
        let (mut state, env, evals) = self.started(FailureGroup::Lifecycle, call_type, &payload.params)?;
        #[rustfmt::skip]
        let request = EvalLifecycleCall { eval_id: &payload.eval_id, extra_headers: payload.params.extra_headers.as_ref(), extra_query: None, timeout: payload.params.timeout, custom_llm_provider: payload.params.custom_llm_provider.as_deref(), kwargs: &payload.params.kwargs };
        let deps = EvalLifecycleDeps {
            configs: &self.configs,
            client: &self.client,
            redactor: &self.redactor,
            request_timeout: self.request_timeout,
        };
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let mut service = EvalLifecycles {
            deps: &deps,
            log: &mut log,
        };
        match action {
            Action::Delete => {
                EvalOperationFailure::reply(service.delete_eval(&request).await, &evals)
            }
            Action::Cancel => {
                EvalOperationFailure::reply(service.cancel_eval(&request).await, &evals)
            }
        }
    }
}
