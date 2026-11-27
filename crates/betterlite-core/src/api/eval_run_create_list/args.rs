use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::{Headers, Object};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;

// create_run's declared arguments, with kwargs standing in for **kwargs
#[derive(Clone, Copy)]
pub struct CreateRunArgs<'call> {
    pub eval_id: &'call str,
    pub data_source: &'call Object,
    pub name: Option<&'call str>,
    pub metadata: Option<&'call Object>,
    pub extra_headers: Option<&'call Headers>,
    pub extra_query: Option<&'call Object>,
    pub extra_body: Option<&'call Object>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'call str>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// list_runs' declared arguments, with the pagination values kept as they arrive
#[derive(Clone, Copy)]
pub struct ListRunsArgs<'call> {
    pub eval_id: &'call str,
    pub limit: Option<i64>,
    pub after: Option<&'call str>,
    pub before: Option<&'call str>,
    pub order: Option<&'call str>,
    pub extra_headers: Option<&'call Headers>,
    pub extra_query: Option<&'call Object>,
    pub timeout: Option<Duration>,
    pub custom_llm_provider: Option<&'call str>,
    pub kwargs: &'call IndexMap<String, Json>,
}

// LiteLLM's local_vars = locals() at create_run's entry, so every argument and **kwargs is in it
pub(crate) fn local_vars_create(request: &CreateRunArgs<'_>) -> IndexMap<String, Json> {
    let pairs = [
        ("eval_id", Some(Json::Text(request.eval_id.to_owned()))),
        ("data_source", Some(json_of(request.data_source))),
        ("name", request.name.map(|text| Json::Text(text.to_owned()))),
        ("metadata", request.metadata.map(json_of)),
        ("extra_headers", request.extra_headers.map(json_of)),
        ("extra_query", request.extra_query.map(json_of)),
        ("extra_body", request.extra_body.map(json_of)),
        ("timeout", request.timeout.map(seconds_of)),
        (
            "custom_llm_provider",
            request
                .custom_llm_provider
                .map(|provider| Json::Text(provider.to_owned())),
        ),
        ("kwargs", Some(Json::Object(request.kwargs.clone()))),
    ];
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.unwrap_or(Json::Null)))
        .collect()
}

// LiteLLM's local_vars = locals() at list_runs' entry, so every argument and **kwargs is in it
pub(crate) fn local_vars_list(request: &ListRunsArgs<'_>) -> IndexMap<String, Json> {
    let limit = request.limit.map(|given| Json::Number(Number::from(given)));
    let after = request.after.map(|text| Json::Text(text.to_owned()));
    let before = request.before.map(|text| Json::Text(text.to_owned()));
    let order = request.order.map(|text| Json::Text(text.to_owned()));
    let pairs = [
        ("eval_id", Some(Json::Text(request.eval_id.to_owned()))),
        ("limit", limit),
        ("after", after),
        ("before", before),
        ("order", order),
        ("extra_headers", request.extra_headers.map(json_of)),
        ("extra_query", request.extra_query.map(json_of)),
        ("timeout", request.timeout.map(seconds_of)),
        (
            "custom_llm_provider",
            request
                .custom_llm_provider
                .map(|provider| Json::Text(provider.to_owned())),
        ),
        ("kwargs", Some(Json::Object(request.kwargs.clone()))),
    ];
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.unwrap_or(Json::Null)))
        .collect()
}

// Python's local holds the timeout in seconds, and the pair fallback writes null for an absent one
fn seconds_of(timeout: Duration) -> Json {
    let Some(seconds) = Number::from_f64(timeout.as_secs_f64()) else {
        return Json::Null;
    };
    Json::Number(seconds)
}

// Json mirrors any serde JSON value, and the objects and headers local_vars holds always convert
fn json_of<T>(value: &T) -> Json
where
    T: Serialize,
{
    let Ok(encoded) = serde_json::to_value(value) else {
        return Json::Null;
    };
    match serde_json::from_value(encoded) {
        Ok(converted) => converted,
        Err(_error) => Json::Null,
    }
}
