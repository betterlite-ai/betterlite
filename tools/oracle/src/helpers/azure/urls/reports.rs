use std::collections::BTreeMap;

use betterlite_azure::azure_urls::{
    EndpointSelection, EnvironmentHeadersRequest, V1_API_VERSIONS, get_azure_v1_image_url,
    get_base_azure_url, resolve_auth_environment, select_base_url_or_endpoint,
    validate_azure_environment,
};
use betterlite_azure::credentials::selection::AzureTokenError;
use betterlite_config::sections::Section;
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw as CredentialRaw};
use betterlite_config::sections::azure_url_policy::{AzureCredentials, Raw};
use secrecy::ExposeSecret;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

use super::sources::NoSources;
use super::{
    CredentialParams, CredentialStep, DeploymentStep, EnvironmentStep, Globals, HeaderStep,
    ImageStep, Input, UrlStep,
};

fn config_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ConfigError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn token_error(error: &AzureTokenError) -> Box<HelperError> {
    Box::new(HelperError {
        class: error.class().to_owned(),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn credentials(
    params: &CredentialParams,
    globals: &Globals,
    environment: &BTreeMap<String, String>,
) -> Result<AzureCredentials, Box<HelperError>> {
    let source = json!({
        "litellm_settings": globals,
        "model_list": [{"litellm_params": params}],
    });
    let raw: Raw = serde_json::from_value(source).or_config_error()?;
    Section::convert(raw, environment)
        .or_config_error()?
        .deployments
        .into_iter()
        .next()
        .ok_or_else(|| config_error("no deployment resolved"))
}

fn token_settings(step: &HeaderStep) -> Result<DeploymentSettings, Box<HelperError>> {
    let source = json!({
        "litellm_settings": {"enable_azure_ad_token_refresh": false},
        "model_list": [{"litellm_params": step.token_params}],
    });
    let raw: CredentialRaw = serde_json::from_value(source).or_config_error()?;
    Section::convert(raw, &step.environment)
        .or_config_error()?
        .deployments
        .into_iter()
        .next()
        .ok_or_else(|| config_error("no deployment resolved"))
}

fn credential_precedence(steps: &[CredentialStep]) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let resolved = credentials(&step.params, &step.globals, &step.environment)?;
        results.push(json!({
            "api_base": resolved.api_base,
            "api_version": resolved.api_version,
            "api_key": resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        }));
    }
    Ok(json!({ "steps": results }))
}

fn deployment_base_url(steps: &[DeploymentStep]) -> serde_json::Value {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let selected = select_base_url_or_endpoint(EndpointSelection {
            azure_endpoint: step.azure_endpoint.clone(),
            base_url: step.base_url.clone(),
        });
        let mut output = BTreeMap::new();
        if let Some(endpoint) = selected.azure_endpoint {
            output.insert("azure_endpoint", endpoint);
        }
        if let Some(base_url) = selected.base_url {
            output.insert("base_url", base_url);
        }
        results.push(json!(output));
    }
    json!({ "steps": results })
}

fn composed_url(step: &UrlStep) -> Result<String, Box<HelperError>> {
    let resolved = credentials(&step.params, &step.globals, &step.environment)?;
    get_base_azure_url(
        resolved.api_base.as_deref(),
        step.params.version.as_deref(),
        step.default_api_version.as_deref(),
        &step.route,
    )
    .or_value_error()
}

fn v1_latest_preview(steps: &[UrlStep]) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        results.push(json!({
            "is_v1": step
                .params
                .version
                .as_deref()
                .is_some_and(|text| V1_API_VERSIONS.contains(&text)),
            "url": composed_url(step)?,
        }));
    }
    Ok(json!({ "steps": results }))
}

fn url_steps(steps: &[UrlStep]) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        results.push(json!({ "url": composed_url(step)? }));
    }
    Ok(json!({ "steps": results }))
}

fn azure_header_selection(steps: &[HeaderStep]) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let resolved = credentials(&step.params, &step.globals, &step.environment)?;
        let settings = token_settings(step)?;
        let sources = NoSources;
        let headers = validate_azure_environment(EnvironmentHeadersRequest {
            headers: step.headers.clone(),
            api_key: resolved.api_key.as_ref(),
            credentials: &settings,
            refresh_enabled: false,
            provider: None,
            sources: &sources,
        })
        .map_err(|error| token_error(&error))?;
        results.push(json!({ "headers": headers }));
    }
    Ok(json!({ "steps": results }))
}

fn azure_env_none_vs_empty(steps: &[EnvironmentStep]) -> serde_json::Value {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let resolved = resolve_auth_environment(step.params.clone(), &step.environment);
        results.push(json!({
            "tenant_id": resolved.tenant_id,
            "client_id": resolved.client_id,
            "client_secret": resolved.client_secret.as_ref().map(ExposeSecret::expose_secret),
            "azure_username": resolved.azure_username,
            "azure_password": resolved.azure_password.as_ref().map(ExposeSecret::expose_secret),
            "azure_scope": resolved.azure_scope,
        }));
    }
    json!({ "steps": results })
}

fn v1_image_resource_reset(steps: &[ImageStep]) -> HelperResult {
    let mut results = Vec::with_capacity(steps.len());
    for step in steps {
        let url = get_azure_v1_image_url(&step.api_base, step.api_version.as_deref(), &step.route)
            .or_value_error()?;
        results.push(json!({ "url": url }));
    }
    Ok(json!({ "steps": results }))
}

pub(super) fn invoke(input: &Input) -> HelperResult {
    match input {
        Input::CredentialPrecedence(steps) => credential_precedence(steps),
        Input::DeploymentBaseUrl(steps) => Ok(deployment_base_url(steps)),
        Input::V1LatestPreview(steps) => v1_latest_preview(steps),
        Input::QueryApiVersionWins(steps) | Input::AppendRouteOnce(steps) => url_steps(steps),
        Input::AzureHeaderSelection(steps) => azure_header_selection(steps),
        Input::AzureEnvNoneVsEmpty(steps) => Ok(azure_env_none_vs_empty(steps)),
        Input::V1ImageResourceReset(steps) => v1_image_resource_reset(steps),
    }
}
