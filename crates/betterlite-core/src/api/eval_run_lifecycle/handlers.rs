use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::evals::types::{CancelRunResponse, Run, RunDeleteResponse};
use betterlite_openai::openai_base_config::Headers;

use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::{EvalConfig, EvalConnection, EvalLogging};
use crate::api::eval_run_http::{cancel_run_handler, delete_run_handler, get_run_handler};

// One run call as B9371's handlers take it: the URL and headers the transform built, the provider config, the resolved timeout and LiteLLM's _is_async
pub struct RunHandlerRequest<'call> {
    pub config: EvalConfig<'call>,
    pub url: &'call str,
    pub headers: &'call Headers,
    pub timeout: Option<Duration>,
    // LiteLLM passes _is_async to every handler; B9371 has one async handler, so the flag stops at this boundary
    pub is_async: bool,
}

// B9371's three handlers behind one seam, so the operations and their tests share the call boundary
pub trait EvalRunHandlers: Send + Sync {
    fn get_run<'call>(
        &'call self,
        request: &'call RunHandlerRequest<'_>,
        logging: &'call mut LoggingState,
    ) -> impl Future<Output = Result<Run, EvalHttpError>> + Send + 'call;

    fn cancel_run<'call>(
        &'call self,
        request: &'call RunHandlerRequest<'_>,
        logging: &'call mut LoggingState,
    ) -> impl Future<Output = Result<CancelRunResponse, EvalHttpError>> + Send + 'call;

    fn delete_run<'call>(
        &'call self,
        request: &'call RunHandlerRequest<'_>,
        logging: &'call mut LoggingState,
    ) -> impl Future<Output = Result<RunDeleteResponse, EvalHttpError>> + Send + 'call;
}

// The production seam: B173's pooled client, and the pre-call environment B249's logging state reads
pub struct HttpRunHandlers<'deps> {
    pub client: &'deps ProxiedClient,
    pub env: &'deps PrePostEnv<'deps>,
}

impl EvalRunHandlers for HttpRunHandlers<'_> {
    #[rustfmt::skip]
    fn get_run<'call>(&'call self, request: &'call RunHandlerRequest<'_>, logging: &'call mut LoggingState) -> impl Future<Output = Result<Run, EvalHttpError>> + Send + 'call {
        let connection = EvalConnection { config: request.config, client: self.client, url: request.url, headers: request.headers };
        let env = self.env;
        async move { get_run_handler(&connection, &mut EvalLogging { state: logging, env }).await }
    }

    #[rustfmt::skip]
    fn cancel_run<'call>(&'call self, request: &'call RunHandlerRequest<'_>, logging: &'call mut LoggingState) -> impl Future<Output = Result<CancelRunResponse, EvalHttpError>> + Send + 'call {
        let connection = EvalConnection { config: request.config, client: self.client, url: request.url, headers: request.headers };
        let env = self.env;
        async move { cancel_run_handler(&connection, &mut EvalLogging { state: logging, env }, request.timeout).await }
    }

    #[rustfmt::skip]
    fn delete_run<'call>(&'call self, request: &'call RunHandlerRequest<'_>, logging: &'call mut LoggingState) -> impl Future<Output = Result<RunDeleteResponse, EvalHttpError>> + Send + 'call {
        let connection = EvalConnection { config: request.config, client: self.client, url: request.url, headers: request.headers };
        let env = self.env;
        async move { delete_run_handler(&connection, &mut EvalLogging { state: logging, env }, request.timeout).await }
    }
}
