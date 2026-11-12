use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::credentials::headers::{
    ClientAuthParams, HeaderLookup, get_azure_request_auth_headers, process_azure_headers,
    redact_azure_auth_headers,
};
use betterlite_azure::credentials::selection::{AzureCredentialRequest, get_azure_ad_token};
use betterlite_azure::credentials::source::CallableTokenProvider;
use betterlite_config::sections::Section;
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

use super::fixture::{FixtureSources, scripted};
use super::{HeaderStep, Input, ResponseSource, ResponseStep, TokenStep, source_error};

pub(super) fn run(input: &Input) -> HelperResult {
    let steps = match input {
        Input::Token(steps) => steps
            .iter()
            .map(token_step)
            .collect::<Result<Vec<_>, _>>()?,
        Input::RequestHeaders(steps) => steps.iter().map(request_header_step).collect(),
        Input::RedactHeaders(steps) => steps
            .iter()
            .map(|step| {
                let output = redact_azure_auth_headers(&header_pairs(&step.headers));
                json!({"kind": "headers", "headers": output.into_iter().collect::<BTreeMap<String, String>>()})
            })
            .collect(),
        Input::ResponseHeaders(steps) => steps.iter().map(response_step).collect(),
    };
    Ok(json!({ "steps": steps }))
}

fn header_pairs(headers: &BTreeMap<String, String>) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn deployment_settings(step: &TokenStep) -> Result<DeploymentSettings, Box<HelperError>> {
    let raw: Raw = serde_json::from_value(json!({
        "litellm_settings": {"enable_azure_ad_token_refresh": step.refresh},
        "model_list": [{"litellm_params": step.params}],
    }))
    .or_config_error()?;
    let settings = Section::convert(raw, &step.environment).or_config_error()?;
    settings
        .deployments
        .into_iter()
        .next()
        .ok_or("no deployment resolved")
        .or_config_error()
}

fn token_step(step: &TokenStep) -> Result<Value, Box<HelperError>> {
    let settings = deployment_settings(step)?;
    let sources = FixtureSources::new(&step.sources);
    let request = AzureCredentialRequest {
        params: &settings,
        refresh_enabled: step.refresh,
        provider: step
            .provider
            .as_ref()
            .map(|script| scripted(script.clone(), &sources.provider_calls)),
        sources: &sources,
    };
    let outcome = match get_azure_ad_token(&request) {
        Ok(Some(token)) => json!({"kind": "token", "token": token.expose_secret()}),
        Ok(None) => json!({"kind": "absent"}),
        Err(error) => json!({"kind": "error", "class": error.class()}),
    };
    Ok(json!({
        "calls": sources.calls(),
        "provider_calls": sources.provider_calls.load(Ordering::Relaxed),
        "outcome": outcome,
        "cache": cache_value(&step.cache, &sources)?,
    }))
}

fn slot_of<'data>(
    held: &mut Vec<Arc<dyn CallableTokenProvider + 'data>>,
    provider: Arc<dyn CallableTokenProvider + 'data>,
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
    let Some(first) = scopes.first() else {
        return Ok(Value::Null);
    };
    let mut cache = RefreshProviderCache::new(sources);
    let mut held = Vec::new();
    let mut slots = Vec::with_capacity(scopes.len());
    for scope in scopes {
        slots.push(slot_of(
            &mut held,
            cache.provider(scope).map_err(source_error)?,
        ));
    }
    let revisit = slot_of(&mut held, cache.provider(first).map_err(source_error)?);
    Ok(json!({"slots": slots, "revisit": revisit}))
}

fn request_header_step(step: &HeaderStep) -> Value {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = ClientAuthParams {
        azure_ad_token: step
            .client
            .azure_ad_token
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        azure_ad_token_provider: step
            .provider
            .as_ref()
            .map(|script| scripted(script.clone(), &calls)),
        api_key: step.client.api_key.clone(),
    };
    let result = get_azure_request_auth_headers(&header_pairs(&step.headers), &client);
    let provider_calls = calls.load(Ordering::Relaxed);
    match result {
        Ok(output) => json!({
            "kind": "headers",
            "headers": json!(output.into_iter().collect::<BTreeMap<String, String>>()),
            "provider_calls": provider_calls,
        }),
        Err(failure) => json!({
            "kind": "error",
            "class": failure.class,
            "provider_calls": provider_calls,
        }),
    }
}

fn response_step(step: &ResponseStep) -> Value {
    let lookup = match step.source {
        ResponseSource::Dict => HeaderLookup::Exact,
        ResponseSource::Httpx => HeaderLookup::CaseInsensitive,
    };
    json!({"kind": "headers", "headers": process_azure_headers(&header_pairs(&step.headers), lookup)})
}
