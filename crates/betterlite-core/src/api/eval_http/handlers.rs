use std::time::Duration;

use betterlite_openai::evals::types::{
    CancelEvalResponse, DeleteEvalResponse, Eval, ListEvalsResponse,
};
use betterlite_openai::openai_base_config::Object;

use super::error::EvalHttpError;
use super::exchange::{EvalConnection, EvalExchange, EvalLogging, EvalMethod, eval_exchange};

// POST the body with the caller's timeout; the pre-call input is the body's name, as LiteLLM's async create logs it
#[rustfmt::skip]
pub async fn create_eval_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, timeout: Option<Duration>, body: &Object) -> Result<Eval, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Post, body: Some(body), query: None, params: None, timeout, input: body.get("name").map_or("", |value| value.as_str().unwrap_or_default()), complete_input_dict: Some(body) };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_create_eval_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// GET the query as params with no timeout, so the client's own deadline holds; the query is also the logged input body
#[rustfmt::skip]
pub async fn list_evals_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, query: &Object) -> Result<ListEvalsResponse, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Get, body: None, query: Some(query), params: None, timeout: None, input: "", complete_input_dict: Some(query) };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_list_evals_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// GET with no params and no timeout, and no body in the pre-call
#[rustfmt::skip]
pub async fn get_eval_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>) -> Result<Eval, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Get, body: None, query: None, params: None, timeout: None, input: "", complete_input_dict: None };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_get_eval_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// POST the body with the caller's timeout; the pre-call input is display_name, which a typed update request never carries
#[rustfmt::skip]
pub async fn update_eval_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, timeout: Option<Duration>, body: &Object) -> Result<Eval, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Post, body: Some(body), query: None, params: None, timeout, input: body.get("display_name").map_or("", |value| value.as_str().unwrap_or_default()), complete_input_dict: Some(body) };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_update_eval_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// DELETE with no body and the caller's timeout
#[rustfmt::skip]
pub async fn delete_eval_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, timeout: Option<Duration>) -> Result<DeleteEvalResponse, EvalHttpError> {
    let exchange = EvalExchange { method: EvalMethod::Delete, body: None, query: None, params: None, timeout, input: "", complete_input_dict: None };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_delete_eval_response(&reply.raw_response()).map_err(EvalHttpError::from)
}

// POST the literal {}, never the body a provider transform built, as LiteLLM's cancel handler does
#[rustfmt::skip]
pub async fn cancel_eval_handler(connection: &EvalConnection<'_>, log: &mut EvalLogging<'_, '_>, timeout: Option<Duration>) -> Result<CancelEvalResponse, EvalHttpError> {
    let body = Object::new();
    let exchange = EvalExchange { method: EvalMethod::Post, body: Some(&body), query: None, params: None, timeout, input: "", complete_input_dict: None };
    let reply = eval_exchange(connection, &exchange, log).await?;
    connection.config.transform_cancel_eval_response(&reply.raw_response()).map_err(EvalHttpError::from)
}
