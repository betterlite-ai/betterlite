use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::credentials::headers::{
    ClientAuthParams, HeaderLookup, get_azure_request_auth_headers, process_azure_headers,
    redact_azure_auth_headers,
};
use betterlite_azure::credentials::selection::{AzureCredentialRequest, get_azure_ad_token};
use betterlite_config::sections::Section;
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::fixture::{FixtureSources, scripted};
use super::{
    HeaderStep, Input, ResponseSource, ResponseStep, TokenStep, helper_error, source_error,
};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Token { steps } => token_steps(steps),
        Input::RequestHeaders { steps } => Ok(header_steps(steps)),
        Input::RedactHeaders { steps } => Ok(redact_steps(steps)),
        Input::ResponseHeaders { steps } => Ok(response_steps(steps)),
    }
}

fn token_steps(steps: &[TokenStep]) -> HelperResult {
    let mut values = Vec::with_capacity(steps.len());
    for step in steps {
        values.push(token_step(step)?);
    }
    Ok(json!({ "steps": values }))
}

fn deployment_settings(step: &TokenStep) -> Result<DeploymentSettings, Box<HelperError>> {
    let raw: Raw = serde_json::from_value(json!({
        "litellm_settings": {"enable_azure_ad_token_refresh": step.refresh},
        "model_list": [{"litellm_params": step.params.view()}],
    }))
    .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
    Section::convert(raw, &step.environment)
        .map_err(|error| helper_error("ConfigError", error.to_string()))?
        .deployments
        .into_iter()
        .next()
        .ok_or_else(|| helper_error("ConfigError", "no deployment resolved".to_owned()))
}

fn token_step(step: &TokenStep) -> Result<Value, Box<HelperError>> {
    let settings = deployment_settings(step)?;
    let sources = FixtureSources::new(&step.sources);
    let caller = step
        .provider
        .as_ref()
        .map(|script| scripted(script.outcome(), &sources.provider_calls));
    let request = AzureCredentialRequest {
        params: &settings,
        refresh_enabled: step.refresh,
        provider: caller,
        sources: &sources,
    };
    let outcome = match get_azure_ad_token(&request) {
        Ok(Some(token)) => json!({"kind": "token", "token": token.expose_secret()}),
        Ok(None) => json!({"kind": "absent"}),
        Err(error) => json!({"kind": "error", "class": error.class()}),
    };
    let cache = if step.cache.is_empty() {
        Value::Null
    } else {
        cache_value(&step.cache, &sources)?
    };
    Ok(json!({
        "calls": sources.calls(),
        "provider_calls": *sources
            .provider_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
        "outcome": outcome,
        "cache": cache,
    }))
}

fn slot_of<'data>(
    held: &mut Vec<Arc<dyn betterlite_azure::credentials::source::CallableTokenProvider + 'data>>,
    provider: Arc<dyn betterlite_azure::credentials::source::CallableTokenProvider + 'data>,
) -> Value {
    if let Some(index) = held
        .iter()
        .position(|existing| Arc::ptr_eq(existing, &provider))
    {
        return json!(index);
    }
    let slot = held.len();
    held.push(provider);
    json!(slot)
}

fn cache_value(scopes: &[String], sources: &FixtureSources) -> Result<Value, Box<HelperError>> {
    let mut cache = RefreshProviderCache::new(sources);
    let mut held: Vec<Arc<dyn betterlite_azure::credentials::source::CallableTokenProvider + '_>> =
        Vec::new();
    let mut slots: Vec<Value> = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let provider = cache.provider(scope).map_err(source_error)?;
        slots.push(slot_of(&mut held, provider));
    }
    let first = scopes
        .first()
        .ok_or_else(|| helper_error("InvalidInput", "cache needs a scope".to_owned()))?;
    let revisit = cache.provider(first).map_err(source_error)?;
    Ok(json!({"slots": slots, "revisit": slot_of(&mut held, revisit)}))
}

fn header_steps(steps: &[HeaderStep]) -> Value {
    let mut values = Vec::with_capacity(steps.len());
    for step in steps {
        values.push(request_header_step(step));
    }
    json!({ "steps": values })
}

fn header_pairs(headers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn request_header_step(step: &HeaderStep) -> Value {
    let calls = Arc::new(Mutex::new(0));
    let caller = step
        .provider
        .as_ref()
        .map(|script| scripted(script.outcome(), &calls));
    let client = ClientAuthParams {
        azure_ad_token: step
            .client
            .azure_ad_token
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        azure_ad_token_provider: caller,
        api_key: step.client.api_key.clone(),
    };
    match get_azure_request_auth_headers(&header_pairs(&step.headers), &client) {
        Ok(output) => json!({
            "kind": "headers",
            "headers": json!(output.into_iter().collect::<BTreeMap<String, String>>()),
            "provider_calls": *calls.lock().unwrap_or_else(PoisonError::into_inner),
        }),
        Err(failure) => json!({
            "kind": "error",
            "class": failure.class,
            "provider_calls": *calls.lock().unwrap_or_else(PoisonError::into_inner),
        }),
    }
}

fn redact_steps(steps: &[HeaderStep]) -> Value {
    let values: Vec<Value> = steps
        .iter()
        .map(|step| {
            json!({
                "kind": "headers",
                "headers": json!(redact_azure_auth_headers(&header_pairs(&step.headers)).into_iter().collect::<BTreeMap<String, String>>()),
            })
        })
        .collect();
    json!({ "steps": values })
}

fn response_steps(steps: &[ResponseStep]) -> Value {
    let mut values = Vec::with_capacity(steps.len());
    for step in steps {
        let lookup = match step.source {
            ResponseSource::Dict => HeaderLookup::Exact,
            ResponseSource::Httpx => HeaderLookup::CaseInsensitive,
        };
        let output = process_azure_headers(&header_pairs(&step.headers), lookup);
        values.push(json!({"kind": "headers", "headers": output}));
    }
    json!({ "steps": values })
}
