use std::future::Future;
use std::time::Duration;

use betterlite_openai::evals::types::{Eval, ListEvalsResponse};
use betterlite_openai::openai_base_config::Object;

use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::{EvalConnection, EvalLogging};
use crate::api::eval_http::handlers::{create_eval_handler, list_evals_handler};

// the inputs BaseLLMHTTPHandler.create_eval_handler takes from create_eval
pub struct CreateEvalHttpCall<'call, 'env> {
    pub connection: EvalConnection<'call>,
    pub logging: EvalLogging<'call, 'env>,
    pub timeout: Duration,
    pub body: &'call Object,
}

// the inputs list_evals_handler takes, plus the timeout LiteLLM hands it and never sends
pub struct ListEvalsHttpCall<'call, 'env> {
    pub connection: EvalConnection<'call>,
    pub logging: EvalLogging<'call, 'env>,
    pub timeout: Duration,
    pub query: &'call Object,
}

// LiteLLM's base_llm_http_handler calls, behind one seam so a test can answer without a provider
pub trait EvalsCreateListHandler: Send + Sync {
    fn create_eval(
        &self,
        call: CreateEvalHttpCall<'_, '_>,
    ) -> impl Future<Output = Result<Eval, EvalHttpError>> + Send;

    fn list_evals(
        &self,
        call: ListEvalsHttpCall<'_, '_>,
    ) -> impl Future<Output = Result<ListEvalsResponse, EvalHttpError>> + Send;
}

// LiteLLM's module-level BaseLLMHTTPHandler(), stateless and shared by every operation
pub struct BaseLlmHttpHandler;

impl EvalsCreateListHandler for BaseLlmHttpHandler {
    async fn create_eval(&self, call: CreateEvalHttpCall<'_, '_>) -> Result<Eval, EvalHttpError> {
        let CreateEvalHttpCall {
            connection,
            mut logging,
            timeout,
            body,
        } = call;
        create_eval_handler(&connection, &mut logging, Some(timeout), body).await
    }

    async fn list_evals(
        &self,
        call: ListEvalsHttpCall<'_, '_>,
    ) -> Result<ListEvalsResponse, EvalHttpError> {
        let ListEvalsHttpCall {
            connection,
            mut logging,
            query,
            ..
        } = call;
        list_evals_handler(&connection, &mut logging, query).await
    }
}

pub const BASE_LLM_HTTP_HANDLER: BaseLlmHttpHandler = BaseLlmHttpHandler;
