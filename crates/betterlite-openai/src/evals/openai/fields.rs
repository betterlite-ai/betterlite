use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::evals::decode::decode_eval_reply;
use crate::evals::error::EvalsError;
use crate::evals::transformation::{EvalsHttpResponse, EvalsRequestContext};
use crate::evals::types::{EvalOrder, EvalOrderBy, ListEvalsParams, ListRunsParams};
use crate::openai_base_config::Object;

pub(super) const EVALS_API_BASE: &str = "https://api.openai.com";

pub(super) const fn present(value: Option<&str>) -> Option<&str> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

pub(super) fn present_secret(secret: Option<&SecretString>) -> Option<&str> {
    match secret {
        Some(value) if !value.expose_secret().is_empty() => Some(value.expose_secret()),
        _ => None,
    }
}

pub(super) fn request_object<T>(request: &T) -> Result<Object, EvalsError>
where
    T: Serialize,
{
    match serde_json::to_value(request).map_err(|source| EvalsError::RequestEncode { source })? {
        Value::Object(fields) => Ok(fields),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(EvalsError::RequestNotObject)
        }
    }
}

pub(super) fn parse_reply<T>(raw_response: &EvalsHttpResponse<'_>) -> Result<T, EvalsError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(decode_eval_reply(raw_response.text.as_bytes())?)
        .map_err(|source| EvalsError::ReplyModel { source })
}

pub(super) fn context_api_base<'call>(context: &EvalsRequestContext<'call>) -> &'call str {
    present(
        context
            .litellm_params
            .get("api_base")
            .and_then(Value::as_str),
    )
    .unwrap_or(EVALS_API_BASE)
}

const fn order_text(order: EvalOrder) -> &'static str {
    match order {
        EvalOrder::Asc => "asc",
        EvalOrder::Desc => "desc",
    }
}

const fn order_by_text(order_by: EvalOrderBy) -> &'static str {
    match order_by {
        EvalOrderBy::CreatedAt => "created_at",
        EvalOrderBy::UpdatedAt => "updated_at",
    }
}

pub(super) fn evals_query(params: &ListEvalsParams) -> Object {
    let mut query = Object::new();
    if let Some(limit) = params.limit.filter(|limit| *limit != 0) {
        query.insert("limit".to_owned(), Value::from(limit));
    }
    if let Some(after) = present(params.after.as_deref()) {
        query.insert("after".to_owned(), Value::from(after));
    }
    if let Some(before) = present(params.before.as_deref()) {
        query.insert("before".to_owned(), Value::from(before));
    }
    if let Some(order) = params.order {
        query.insert("order".to_owned(), Value::from(order_text(order)));
    }
    if let Some(order_by) = params.order_by {
        query.insert("order_by".to_owned(), Value::from(order_by_text(order_by)));
    }
    query
}

pub(super) fn runs_query(params: &ListRunsParams) -> Object {
    // runs forward the same four query parameters as evals, and never order_by
    let shared = ListEvalsParams {
        limit: params.limit,
        after: params.after.clone(),
        before: params.before.clone(),
        order: params.order,
        order_by: None,
    };
    evals_query(&shared)
}
