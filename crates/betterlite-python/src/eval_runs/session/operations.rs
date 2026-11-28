use betterlite_core::api::eval_http::exchange::EvalLogging;
use betterlite_core::api::eval_lifecycle::EvalLifecycleConfigLookup as _;
use betterlite_core::api::eval_run_create_list::EvalRunCreateList;
use betterlite_core::api::eval_run_create_list::args::{CreateRunArgs, ListRunsArgs};
use betterlite_core::api::eval_run_lifecycle::error::RunAction;
use betterlite_core::api::eval_run_lifecycle::handlers::HttpRunHandlers;
use betterlite_core::api::eval_run_lifecycle::{
    EvalRunDeps, EvalRunRequest, acancel_run, adelete_run, aget_run,
};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::EvalCore;
use super::logging::{self, SILENT_SINK};
use crate::eval_runs::configs::EvalConfigs;
use crate::eval_runs::convert::{CreateRunPayload, ListRunPayload, Params, RunIdPayload};
use crate::eval_runs::error::{EvalOperationFailure, FailureGroup, SessionError};
use crate::eval_runs::transport::HttpRunTransport;

// Each arm's payload stays on the heap, so the enum holds one word plus its tag.
pub(super) enum RunCall {
    CreateList(Box<CreateListCall>),
    Get(Box<RunIdPayload>),
    Cancel(Box<RunIdPayload>),
    Delete(Box<RunIdPayload>),
}

// create_run and list_runs share the create/list service; each arm is one of the argument types it takes.
pub(super) enum CreateListCall {
    Create(CreateRunPayload),
    List(ListRunPayload),
}

impl CreateListCall {
    const fn label(&self) -> (FailureGroup, &'static str, &Params) {
        match self {
            Self::Create(payload) => (FailureGroup::CreateList, "create_run", &payload.params),
            Self::List(payload) => (FailureGroup::CreateList, "list_runs", &payload.params),
        }
    }
}

impl EvalCore {
    pub(super) async fn execute(&self, call: RunCall) -> Result<String, EvalOperationFailure> {
        match call {
            RunCall::CreateList(request) => self.request(&request).await,
            RunCall::Get(payload) => self.lifecycle(RunAction::Get, &payload).await,
            RunCall::Cancel(payload) => self.lifecycle(RunAction::Cancel, &payload).await,
            RunCall::Delete(payload) => self.lifecycle(RunAction::Delete, &payload).await,
        }
    }

    // What every run call does before its service runs: build the logging state and the environment its pre-call reads.
    fn started<'core>(
        &'core self,
        group: FailureGroup,
        call_type: &str,
        params: &Params,
    ) -> Result<(LoggingState, PrePostEnv<'core>), EvalOperationFailure> {
        let state = logging::state(self, call_type, &params.kwargs).map_err(|source| {
            EvalOperationFailure::Mapped {
                class: group.logging(source, params.provider(), &self.redactor),
                provider: params.provider().to_owned(),
            }
        })?;
        Ok((state, logging::env(&self.redactor, &SILENT_SINK)))
    }

    // LiteLLM's create_run and list_runs share one driver; each arm builds the arguments its service method takes.
    async fn request(&self, call: &CreateListCall) -> Result<String, EvalOperationFailure> {
        let (group, call_type, params) = call.label();
        let (mut state, env) = self.started(group, call_type, params)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SessionError::from)?;
        #[rustfmt::skip]
        let service = EvalRunCreateList { providers: EvalConfigs::new(self.settings.clone(), &self.environment), transport: HttpRunTransport { client: &self.client }, redactor, request_timeout: self.request_timeout };
        match call {
            CreateListCall::Create(payload) => {
                #[rustfmt::skip]
                let request = CreateRunArgs { eval_id: &payload.eval_id, data_source: &payload.data_source, name: payload.name.as_deref(), metadata: payload.metadata.as_ref(), extra_headers: payload.params.extra_headers.as_ref(), extra_query: payload.params.extra_query.as_ref(), extra_body: payload.extra_body.as_ref(), timeout: payload.params.timeout, custom_llm_provider: payload.params.custom_llm_provider.as_deref(), kwargs: &payload.params.kwargs };
                #[rustfmt::skip]
                let reply = service.acreate_run(request, EvalLogging { state: &mut state, env: &env }).await;
                EvalOperationFailure::reply(reply, payload.params.provider())
            }
            CreateListCall::List(payload) => {
                #[rustfmt::skip]
                let request = ListRunsArgs { eval_id: &payload.eval_id, limit: payload.limit, after: payload.after.as_deref(), before: payload.before.as_deref(), order: payload.order.as_deref(), extra_headers: payload.params.extra_headers.as_ref(), extra_query: payload.params.extra_query.as_ref(), timeout: payload.params.timeout, custom_llm_provider: payload.params.custom_llm_provider.as_deref(), kwargs: &payload.params.kwargs };
                #[rustfmt::skip]
                let reply = service.alist_runs(request, EvalLogging { state: &mut state, env: &env }).await;
                EvalOperationFailure::reply(reply, payload.params.provider())
            }
        }
    }

    // get_run, cancel_run and delete_run share the lifecycle preparation and differ in the transform and handler.
    async fn lifecycle(
        &self,
        action: RunAction,
        payload: &RunIdPayload,
    ) -> Result<String, EvalOperationFailure> {
        let operation = match action {
            RunAction::Get => "get_run",
            RunAction::Cancel => "cancel_run",
            RunAction::Delete => "delete_run",
        };
        let (mut state, env) = self.started(FailureGroup::Lifecycle, operation, &payload.params)?;
        let provider = payload.params.provider();
        let config = EvalConfigs::new(self.settings.clone(), &self.environment)
            .eval_config(provider)
            .unwrap_or_default();
        let handlers = HttpRunHandlers {
            client: &self.client,
            env: &env,
        };
        #[rustfmt::skip]
        let deps = EvalRunDeps { handler: &handlers, config: &config, redactor: &self.redactor, request_timeout: self.request_timeout };
        #[rustfmt::skip]
        let request = EvalRunRequest { eval_id: &payload.eval_id, run_id: &payload.run_id, extra_headers: payload.params.extra_headers.as_ref(), extra_query: payload.params.extra_query.as_ref(), timeout: payload.params.timeout, custom_llm_provider: payload.params.custom_llm_provider.as_deref(), kwargs: &payload.params.kwargs };
        match action {
            RunAction::Get => {
                EvalOperationFailure::reply(aget_run(&deps, &request, &mut state).await, provider)
            }
            RunAction::Cancel => EvalOperationFailure::reply(
                acancel_run(&deps, &request, &mut state).await,
                provider,
            ),
            RunAction::Delete => EvalOperationFailure::reply(
                adelete_run(&deps, &request, &mut state).await,
                provider,
            ),
        }
    }
}
