use std::future::Future;
use std::time::Duration;

use betterlite_openai::evals::types::Eval;
use betterlite_openai::openai_base_config::Object;

use crate::api::eval_create_list::handler::BaseLlmHttpHandler;
use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::{EvalConnection, EvalLogging};
use crate::api::eval_http::handlers::{get_eval_handler, update_eval_handler};

// the inputs BaseLLMHTTPHandler.get_eval_handler takes from get_eval, timeout included even though a GET never sends it
pub struct GetEvalHttpCall<'call, 'env> {
    pub connection: EvalConnection<'call>,
    pub logging: EvalLogging<'call, 'env>,
    pub timeout: Duration,
}

// the inputs update_eval_handler takes: the transformed body and the resolved timeout the POST sends
pub struct UpdateEvalHttpCall<'call, 'env> {
    pub connection: EvalConnection<'call>,
    pub logging: EvalLogging<'call, 'env>,
    pub timeout: Duration,
    pub body: &'call Object,
}

// LiteLLM's base_llm_http_handler calls, behind one seam so a test can answer without a provider
pub trait EvalsGetUpdateHandler: Send + Sync {
    fn get_eval(
        &self,
        call: GetEvalHttpCall<'_, '_>,
    ) -> impl Future<Output = Result<Eval, EvalHttpError>> + Send;

    fn update_eval(
        &self,
        call: UpdateEvalHttpCall<'_, '_>,
    ) -> impl Future<Output = Result<Eval, EvalHttpError>> + Send;
}

// B3421's module-level BaseLLMHTTPHandler(), which carries no state and serves every eval operation
impl EvalsGetUpdateHandler for BaseLlmHttpHandler {
    async fn get_eval(&self, call: GetEvalHttpCall<'_, '_>) -> Result<Eval, EvalHttpError> {
        let GetEvalHttpCall {
            connection,
            mut logging,
            ..
        } = call;
        get_eval_handler(&connection, &mut logging).await
    }

    async fn update_eval(&self, call: UpdateEvalHttpCall<'_, '_>) -> Result<Eval, EvalHttpError> {
        let UpdateEvalHttpCall {
            connection,
            mut logging,
            timeout,
            body,
        } = call;
        update_eval_handler(&connection, &mut logging, Some(timeout), body).await
    }
}
