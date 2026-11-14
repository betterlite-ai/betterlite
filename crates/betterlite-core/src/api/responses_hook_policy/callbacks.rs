use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_parameters::requested::RESPONSES_OPTIONAL_PARAMS;

use super::error::DeploymentHookError;
use super::python_value::python_str;

pub const RESPONSES_CALL_TYPE: &str = "responses";

// the source compares the callback class's method against CustomLogger's own; a registration that
// carries an override, inherited or not, records that here
#[derive(Clone)]
pub struct PolicyCallback {
    pub name: String,
    pub overrides_pre_call_deployment_hook: bool,
    pub agentic: Option<Arc<dyn AgenticCallback>>,
}

#[derive(Clone)]
pub enum CallbackValue {
    Object(PolicyCallback),
    Named(String),
    Other,
}

#[derive(Clone, Copy)]
pub struct CallbackLists<'a> {
    pub global: &'a [CallbackValue],
    pub dynamic: Option<&'a [CallbackValue]>,
}

pub trait NamedCallbackResolver: Send + Sync {
    #[must_use]
    fn resolve(&self, name: &str) -> Option<PolicyCallback>;
}

// B157's deployment-hook dispatcher behind a typed port; None is the source's null hook result
#[async_trait]
pub trait DeploymentHookDispatcher: Send + Sync {
    async fn pre_call_deployment_hook(
        &self,
        request: IndexMap<String, Json>,
        call_type: &str,
    ) -> Result<Option<IndexMap<String, Json>>, DeploymentHookError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponsesHookRequest {
    pub model: String,
    pub input: Json,
    pub custom_llm_provider: String,
    pub optional_params: IndexMap<String, Json>,
    pub generic_params: IndexMap<String, Json>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponsesHookOutcome {
    pub model: String,
    pub input: Json,
    pub custom_llm_provider: String,
    pub optional_params: IndexMap<String, Json>,
    pub generic_params: IndexMap<String, Json>,
}

#[must_use]
pub fn responses_api_optional_request_param_names() -> BTreeSet<String> {
    let names = RESPONSES_OPTIONAL_PARAMS
        .split_whitespace()
        .map(str::to_owned);
    names.collect()
}

// global callbacks come first, then the dynamic ones, with duplicates kept and non-loggers skipped
#[must_use]
pub fn custom_logger_callbacks(
    lists: &CallbackLists<'_>,
    resolver: &dyn NamedCallbackResolver,
) -> Vec<PolicyCallback> {
    lists
        .global
        .iter()
        .chain(lists.dynamic.unwrap_or(&[]))
        .filter_map(|value| resolve_callback(value, resolver))
        .collect()
}

fn resolve_callback(
    value: &CallbackValue,
    resolver: &dyn NamedCallbackResolver,
) -> Option<PolicyCallback> {
    match value {
        CallbackValue::Object(callback) => Some(callback.clone()),
        CallbackValue::Named(name) => resolver.resolve(name),
        CallbackValue::Other => None,
    }
}

#[must_use]
pub fn has_pre_call_deployment_hook(
    lists: &CallbackLists<'_>,
    resolver: &dyn NamedCallbackResolver,
) -> bool {
    let callbacks = custom_logger_callbacks(lists, resolver);
    callbacks
        .iter()
        .any(|callback| callback.overrides_pre_call_deployment_hook)
}

#[must_use]
pub fn has_agentic_completion_hook(
    lists: &CallbackLists<'_>,
    resolver: &dyn NamedCallbackResolver,
) -> bool {
    let callbacks = custom_logger_callbacks(lists, resolver);
    callbacks.iter().any(|callback| {
        callback
            .agentic
            .as_ref()
            .is_some_and(|agentic| agentic.overrides().should_run)
    })
}

pub async fn run_sync_responses_pre_call_deployment_hook(
    request: ResponsesHookRequest,
    callbacks: &CallbackLists<'_>,
    resolver: &dyn NamedCallbackResolver,
    dispatcher: &dyn DeploymentHookDispatcher,
) -> Result<ResponsesHookOutcome, DeploymentHookError> {
    if !has_pre_call_deployment_hook(callbacks, resolver) {
        return Ok(original_outcome(request));
    }
    let hook_request = build_hook_input(&request);
    let Some(modified) = dispatcher
        .pre_call_deployment_hook(hook_request, RESPONSES_CALL_TYPE)
        .await?
    else {
        return Ok(original_outcome(request));
    };
    Ok(split(modified, request))
}

fn build_hook_input(request: &ResponsesHookRequest) -> IndexMap<String, Json> {
    let mut input = request.generic_params.clone();
    input.extend(request.optional_params.clone());
    input.insert("model".to_owned(), Json::Text(request.model.clone()));
    input.insert("input".to_owned(), request.input.clone());
    input.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(request.custom_llm_provider.clone()),
    );
    input
}

fn split(modified: IndexMap<String, Json>, request: ResponsesHookRequest) -> ResponsesHookOutcome {
    let optional_names = responses_api_optional_request_param_names();
    let mut optional_params = request.optional_params;
    let mut generic_params = request.generic_params;
    for (key, value) in &modified {
        if optional_names.contains(key) {
            optional_params.insert(key.clone(), value.clone());
        } else if !matches!(key.as_str(), "model" | "input" | "custom_llm_provider") {
            generic_params.insert(key.clone(), value.clone());
        }
    }
    let model = modified.get("model").map_or(request.model, python_str);
    let input = modified.get("input").cloned().unwrap_or(request.input);
    let custom_llm_provider = modified
        .get("custom_llm_provider")
        .map_or(request.custom_llm_provider, python_str);
    ResponsesHookOutcome {
        model,
        input,
        custom_llm_provider,
        optional_params,
        generic_params,
    }
}

fn original_outcome(request: ResponsesHookRequest) -> ResponsesHookOutcome {
    ResponsesHookOutcome {
        model: request.model,
        input: request.input,
        custom_llm_provider: request.custom_llm_provider,
        optional_params: request.optional_params,
        generic_params: request.generic_params,
    }
}
