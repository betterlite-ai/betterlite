mod tests;

use std::time::Duration;

use betterlite_openai::evals::types::{
    CancelRunResponse, ListRunsResponse, Run, RunDeleteResponse,
};
use betterlite_openai::openai_base_config::Object;

use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::{
    EvalConnection, EvalExchange, EvalLogging, EvalMethod, eval_exchange,
};

// POST the body with the caller's timeout; the pre-call input is the body's name, as LiteLLM's async create_run logs it
#[rustfmt::skip]
pub async fn create_run_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, timeout: Option<Duration>, body: &Object) -> Result<Run, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Post, body: Some(body), query: None, timeout, input: body.get("name").map_or("", |value| value.as_str().unwrap_or_default()), complete_input_dict: Some(body) };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_create_run_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// GET the query as params with no timeout; LiteLLM logs the query under `params`, and eval_exchange has no field that reaches that key yet
#[rustfmt::skip]
pub async fn list_runs_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, query: &Object) -> Result<ListRunsResponse, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Get, body: None, query: Some(query), timeout: None, input: "", complete_input_dict: None };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_list_runs_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// GET with no params and no timeout, so the client's own deadline holds
#[rustfmt::skip]
pub async fn get_run_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>) -> Result<Run, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Get, body: None, query: None, timeout: None, input: "", complete_input_dict: None };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_get_run_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// POST the literal {}, never the body transform_cancel_run_request built, with the caller's timeout; the URL already ends in /cancel
#[rustfmt::skip]
pub async fn cancel_run_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, timeout: Option<Duration>) -> Result<CancelRunResponse, EvalHttpError> {
    let body = Object::new();
    let exchange = EvalExchange { method: EvalMethod::Post, body: Some(&body), query: None, timeout, input: "", complete_input_dict: None };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_cancel_run_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// DELETE with no body and the caller's timeout, dropping whatever body transform_delete_run_request built
#[rustfmt::skip]
pub async fn delete_run_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, timeout: Option<Duration>) -> Result<RunDeleteResponse, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Delete, body: None, query: None, timeout, input: "", complete_input_dict: None };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_delete_run_response(&reply.raw_response()).map_err(EvalHttpError::from)
}
