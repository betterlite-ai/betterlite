use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::azure_urls::{
    EndpointSelection, EnvironmentHeadersRequest, get_azure_v1_image_url, get_base_azure_url,
    is_azure_v1_api_version, resolve_env_var, select_base_url_or_endpoint,
    validate_azure_environment,
};
use betterlite_azure::credentials::selection::AzureTokenError;
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::Section;
use betterlite_config::sections::azure_credentials::{DeploymentSettings, Raw as CredentialRaw};
use betterlite_config::sections::azure_url_policy::{AzureCredentials, Raw};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

mod tests;

const ENV_KEYS: [(&str, &str); 6] = [
    ("tenant_id", "AZURE_TENANT_ID"),
    ("client_id", "AZURE_CLIENT_ID"),
    ("client_secret", "AZURE_CLIENT_SECRET"),
    ("azure_username", "AZURE_USERNAME"),
    ("azure_password", "AZURE_PASSWORD"),
    ("azure_scope", "AZURE_SCOPE"),
];

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialParams {
    #[serde(rename = "api_base")]
    base: Option<String>,
    #[serde(rename = "api_key")]
    key: Option<String>,
    #[serde(rename = "api_version")]
    version: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Globals {
    api_base: Option<String>,
    api_key: Option<String>,
    azure_key: Option<String>,
    api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialStep {
    #[serde(default)]
    params: CredentialParams,
    #[serde(default)]
    globals: Globals,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    #[serde(default)]
    params: CredentialParams,
    #[serde(default)]
    globals: Globals,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    route: String,
    #[serde(default)]
    default_api_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeploymentStep {
    #[serde(default)]
    azure_endpoint: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentStep {
    #[serde(default)]
    params: BTreeMap<String, Option<String>>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderStep {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    params: CredentialParams,
    #[serde(default)]
    globals: Globals,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    token_params: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageStep {
    api_base: String,
    #[serde(default)]
    api_version: Option<String>,
    route: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    CredentialPrecedence(Vec<CredentialStep>),
    DeploymentBaseUrl(Vec<DeploymentStep>),
    V1LatestPreview(Vec<UrlStep>),
    QueryApiVersionWins(Vec<UrlStep>),
    AppendRouteOnce(Vec<UrlStep>),
    AzureHeaderSelection(Vec<HeaderStep>),
    AzureEnvNoneVsEmpty(Vec<EnvironmentStep>),
    V1ImageResourceReset(Vec<ImageStep>),
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        invoke(input)
    }
}

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
            "is_v1": is_azure_v1_api_version(step.params.version.as_deref()),
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
        let mut output = BTreeMap::new();
        for (param, variable) in ENV_KEYS {
            output.insert(
                param.to_owned(),
                resolve_env_var(&step.params, param, variable, &step.environment),
            );
        }
        results.push(json!(output));
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

fn invoke(input: &Input) -> HelperResult {
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

pub(super) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<secrecy::SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}
