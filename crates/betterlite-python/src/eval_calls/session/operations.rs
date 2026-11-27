use betterlite_core::api::eval_create_list::call::{EvalCreation, EvalListing};
use betterlite_core::api::eval_create_list::error::EvalsCreateListError;
use betterlite_core::api::eval_create_list::handler::BASE_LLM_HTTP_HANDLER;
use betterlite_core::api::eval_create_list::service::EvalsCreateListService;
use betterlite_core::api::eval_get_update::call::{EvalRetrieval, EvalUpdate};
use betterlite_core::api::eval_get_update::error::EvalsGetUpdateError;
use betterlite_core::api::eval_get_update::service::EvalsGetUpdateService;
use betterlite_core::api::eval_http::exchange::EvalLogging;
use betterlite_core::api::eval_lifecycle::error::EvalLifecycleError;
use betterlite_core::api::eval_lifecycle::{EvalLifecycleDeps, EvalLifecycles};

use super::EvalCore;
use super::logging::{self, SilentSink};
use crate::eval_calls::error::EvalOperationFailure;
use crate::eval_calls::payload::{
    CreatePayload, GetPayload, LifecyclePayload, ListPayload, UpdatePayload,
};

// The operation one Python call asks for, named after the session method that took it.
#[derive(Clone, Copy)]
pub(super) enum Mode {
    Create,
    List,
    Get,
    Update,
    Delete,
    Cancel,
}

// The two lifecycle calls differ only in the transform and handler the service picks.
#[derive(Clone, Copy)]
enum Lifecycle {
    Delete,
    Cancel,
}

impl EvalCore {
    // Every eval call runs the same way: read the payload, build the logging state, then let the core service do the work.
    pub(super) async fn execute(
        &self,
        mode: Mode,
        text: &str,
    ) -> Result<String, EvalOperationFailure> {
        match mode {
            Mode::Create => self.create(text).await,
            Mode::List => self.list(text).await,
            Mode::Get => self.get(text).await,
            Mode::Update => self.update(text).await,
            Mode::Delete => self.lifecycle(text, Lifecycle::Delete).await,
            Mode::Cancel => self.lifecycle(text, Lifecycle::Cancel).await,
        }
    }

    async fn create(&self, text: &str) -> Result<String, EvalOperationFailure> {
        let payload = CreatePayload::parse(text)?;
        let params = payload.params.call();
        let mut state = logging::call_logging(self, "create_eval", &payload.params.kwargs)
            .map_err(|source| {
                EvalOperationFailure::create_list(
                    EvalsCreateListError::Logging(source),
                    &params,
                    &self.redactor,
                )
            })?;
        let sink = SilentSink;
        let env = logging::pre_post_env(self, &sink);
        let call = payload.call();
        let service = EvalsCreateListService {
            handler: &BASE_LLM_HTTP_HANDLER,
            configs: &self.configs,
            client: &self.client,
            core: &self.core,
            redactor: &self.redactor,
        };
        let creation = EvalCreation {
            call: &call,
            logging: &mut state,
            env: &env,
        };
        let reply = service.create_eval(creation).await;
        EvalOperationFailure::create_list_reply(reply, &params)
    }

    async fn list(&self, text: &str) -> Result<String, EvalOperationFailure> {
        let payload = ListPayload::parse(text)?;
        let params = payload.params.call();
        let mut state = logging::call_logging(self, "list_evals", &payload.params.kwargs).map_err(
            |source| {
                EvalOperationFailure::create_list(
                    EvalsCreateListError::Logging(source),
                    &params,
                    &self.redactor,
                )
            },
        )?;
        let sink = SilentSink;
        let env = logging::pre_post_env(self, &sink);
        let call = payload.call();
        let service = EvalsCreateListService {
            handler: &BASE_LLM_HTTP_HANDLER,
            configs: &self.configs,
            client: &self.client,
            core: &self.core,
            redactor: &self.redactor,
        };
        let listing = EvalListing {
            call: &call,
            logging: &mut state,
            env: &env,
        };
        let reply = service.list_evals(listing).await;
        EvalOperationFailure::create_list_reply(reply, &params)
    }

    async fn get(&self, text: &str) -> Result<String, EvalOperationFailure> {
        let payload = GetPayload::parse(text)?;
        let params = payload.params.call();
        let mut state =
            logging::call_logging(self, "get_eval", &payload.params.kwargs).map_err(|source| {
                EvalOperationFailure::get_update(
                    EvalsGetUpdateError::Logging(source),
                    &params,
                    &self.redactor,
                )
            })?;
        let sink = SilentSink;
        let env = logging::pre_post_env(self, &sink);
        let call = payload.call();
        let service = EvalsGetUpdateService {
            handler: &BASE_LLM_HTTP_HANDLER,
            configs: &self.configs,
            client: &self.client,
            core: &self.core,
            redactor: &self.redactor,
        };
        let retrieval = EvalRetrieval {
            call: &call,
            logging: &mut state,
            env: &env,
        };
        let reply = service.get_eval(retrieval).await;
        EvalOperationFailure::get_update_reply(reply, &params)
    }

    async fn update(&self, text: &str) -> Result<String, EvalOperationFailure> {
        let payload = UpdatePayload::parse(text)?;
        let params = payload.params.call();
        let mut state = logging::call_logging(self, "update_eval", &payload.params.kwargs)
            .map_err(|source| {
                EvalOperationFailure::get_update(
                    EvalsGetUpdateError::Logging(source),
                    &params,
                    &self.redactor,
                )
            })?;
        let sink = SilentSink;
        let env = logging::pre_post_env(self, &sink);
        let call = payload.call();
        let service = EvalsGetUpdateService {
            handler: &BASE_LLM_HTTP_HANDLER,
            configs: &self.configs,
            client: &self.client,
            core: &self.core,
            redactor: &self.redactor,
        };
        let update = EvalUpdate {
            call: &call,
            logging: &mut state,
            env: &env,
        };
        let reply = service.update_eval(update).await;
        EvalOperationFailure::get_update_reply(reply, &params)
    }

    async fn lifecycle(
        &self,
        text: &str,
        action: Lifecycle,
    ) -> Result<String, EvalOperationFailure> {
        let payload = LifecyclePayload::parse(text)?;
        let params = payload.params.call();
        let call_type = match action {
            Lifecycle::Delete => "delete_eval",
            Lifecycle::Cancel => "cancel_eval",
        };
        let mut state =
            logging::call_logging(self, call_type, &payload.params.kwargs).map_err(|source| {
                EvalOperationFailure::lifecycle(
                    EvalLifecycleError::Logging(source),
                    &params,
                    &self.redactor,
                )
            })?;
        let sink = SilentSink;
        let env = logging::pre_post_env(self, &sink);
        let call = payload.call();
        let deps = EvalLifecycleDeps {
            configs: &self.configs,
            client: &self.client,
            redactor: &self.redactor,
            request_timeout: self.request_timeout,
        };
        let mut log = EvalLogging {
            state: &mut state,
            env: &env,
        };
        let mut service = EvalLifecycles {
            deps: &deps,
            log: &mut log,
        };
        match action {
            Lifecycle::Delete => {
                let reply = service.delete_eval(&call).await;
                EvalOperationFailure::lifecycle_reply(reply, &params)
            }
            Lifecycle::Cancel => {
                let reply = service.cancel_eval(&call).await;
                EvalOperationFailure::lifecycle_reply(reply, &params)
            }
        }
    }
}
