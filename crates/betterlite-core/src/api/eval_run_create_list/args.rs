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
    let mut vars = IndexMap::new();
    vars.insert("eval_id".to_owned(), Json::Text(request.eval_id.to_owned()));
    vars.insert("data_source".to_owned(), json_of(request.data_source));
    vars.insert(
        "name".to_owned(),
        request
            .name
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    vars.insert(
        "metadata".to_owned(),
        request.metadata.map_or(Json::Null, json_of),
    );
    vars.insert(
        "extra_headers".to_owned(),
        request.extra_headers.map_or(Json::Null, json_of),
    );
    vars.insert(
        "extra_query".to_owned(),
        request.extra_query.map_or(Json::Null, json_of),
    );
    vars.insert(
        "extra_body".to_owned(),
        request.extra_body.map_or(Json::Null, json_of),
    );
    vars.insert("timeout".to_owned(), seconds_of(request.timeout));
    vars.insert(
        "custom_llm_provider".to_owned(),
        request
            .custom_llm_provider
            .map_or(Json::Null, |provider| Json::Text(provider.to_owned())),
    );
    vars.insert("kwargs".to_owned(), Json::Object(request.kwargs.clone()));
    vars
}

// LiteLLM's local_vars = locals() at list_runs' entry, so every argument and **kwargs is in it
pub(crate) fn local_vars_list(request: &ListRunsArgs<'_>) -> IndexMap<String, Json> {
    let mut vars = IndexMap::new();
    vars.insert("eval_id".to_owned(), Json::Text(request.eval_id.to_owned()));
    vars.insert(
        "limit".to_owned(),
        request
            .limit
            .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
    );
    vars.insert(
        "after".to_owned(),
        request
            .after
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    vars.insert(
        "before".to_owned(),
        request
            .before
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    vars.insert(
        "order".to_owned(),
        request
            .order
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
    vars.insert(
        "extra_headers".to_owned(),
        request.extra_headers.map_or(Json::Null, json_of),
    );
    vars.insert(
        "extra_query".to_owned(),
        request.extra_query.map_or(Json::Null, json_of),
    );
    vars.insert("timeout".to_owned(), seconds_of(request.timeout));
    vars.insert(
        "custom_llm_provider".to_owned(),
        request
            .custom_llm_provider
            .map_or(Json::Null, |provider| Json::Text(provider.to_owned())),
    );
    vars.insert("kwargs".to_owned(), Json::Object(request.kwargs.clone()));
    vars
}

// Python's local holds the timeout it was given, as a number of seconds; an absent one is null
fn seconds_of(timeout: Option<Duration>) -> Json {
    let Some(given) = timeout else {
        return Json::Null;
    };
    match Number::from_f64(given.as_secs_f64()) {
        Some(seconds) => Json::Number(seconds),
        None => Json::Null,
    }
}

// Json mirrors any serde JSON value, and the objects and headers local_vars holds always convert
fn json_of<T: Serialize>(value: &T) -> Json {
    let Ok(encoded) = serde_json::to_value(value) else {
        return Json::Null;
    };
    match serde_json::from_value(encoded) {
        Ok(converted) => converted,
        Err(_error) => Json::Null,
    }
}
