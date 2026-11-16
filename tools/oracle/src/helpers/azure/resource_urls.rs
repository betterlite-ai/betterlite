mod sources;
mod tests;

use std::collections::BTreeMap;

use betterlite_azure::resource_urls::{
    AzureResource, ResourceAuth, UrlRequest, VIDEO_SUPPORTED_PARAMS, complete_url,
    validate_environment, video_map_params, video_validate_environment,
};
use betterlite_config::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings};
use betterlite_openai::videos::create::VideoCreate;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use sources::NoSources;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Module {
    api_base: Option<String>,
    api_key: Option<String>,
    azure_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlStep {
    api_base: Option<String>,
    api_version: Option<String>,
    module: Module,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {
    api_key: Option<String>,
    azure_ad_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Resource {
    Container,
    Video,
    VectorStore,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthStep {
    resource: Resource,
    headers: BTreeMap<String, String>,
    api_key: Option<String>,
    params: Option<Params>,
    module: Module,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VideoStep {
    options: VideoCreate,
    // drop_params has no effect on the mapping, so nothing reads it
    #[serde(rename = "drop_params")]
    _drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlsAndAuth {
    urls: Vec<UrlStep>,
    auth: Vec<AuthStep>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    ContainerApiVersionQuery(Vec<UrlStep>),
    ContainerNormalizedBase(Vec<UrlStep>),
    VideoSupportedFields(Vec<VideoStep>),
    VideoAuthAndUrl(UrlsAndAuth),
    VectorSearchAzureBase(UrlsAndAuth),
}

fn url_report(
    resource: AzureResource,
    step: &UrlStep,
) -> Result<serde_json::Value, Box<HelperError>> {
    let fallback = step
        .module
        .api_base
        .clone()
        .filter(|text| !text.is_empty())
        .or_else(|| step.environment.get("AZURE_API_BASE").cloned());
    let request = UrlRequest {
        api_base: step.api_base.as_deref(),
        fallback_api_base: fallback.as_deref(),
        api_version: step.api_version.as_deref(),
    };
    let url = complete_url(resource, &request).or_value_error()?;
    Ok(json!({ "url": url, "params_api_version": step.api_version }))
}

fn fallback_api_key(step: &AuthStep) -> Option<SecretString> {
    [
        step.module.api_key.as_ref(),
        step.module.azure_key.as_ref(),
        step.environment.get("AZURE_OPENAI_API_KEY"),
        step.environment.get("AZURE_API_KEY"),
    ]
    .into_iter()
    .flatten()
    .find(|text| !text.is_empty())
    .cloned()
    .map(SecretString::from)
}

fn token_settings(step: &AuthStep) -> DeploymentSettings {
    // GenericLiteLLMParams(api_key=api_key) is all a container sees, so its params never reach the token
    let supplied = step
        .params
        .as_ref()
        .filter(|_| !matches!(step.resource, Resource::Container));
    let token = supplied
        .and_then(|params| params.azure_ad_token.clone())
        .filter(|text| !text.is_empty())
        .or_else(|| step.environment.get("AZURE_AD_TOKEN").cloned());
    DeploymentSettings {
        azure_ad_token: token.map(SecretString::from),
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: DEFAULT_SCOPE.to_owned(),
        azure_username: None,
        azure_password: None,
    }
}

fn auth_report(step: &AuthStep) -> Result<serde_json::Value, Box<HelperError>> {
    let fallback = fallback_api_key(step);
    let credentials = token_settings(step);
    let auth = ResourceAuth {
        headers: step.headers.clone(),
        fallback_api_key: fallback.as_ref(),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &NoSources,
    };
    let positional = step.api_key.clone().map(SecretString::from);
    let mut params_api_key = step
        .params
        .as_ref()
        .and_then(|params| params.api_key.clone())
        .map(SecretString::from);
    let headers = match step.resource {
        Resource::Container => validate_environment(auth, positional.as_ref()),
        Resource::Video => {
            video_validate_environment(auth, positional.as_ref(), &mut params_api_key)
        }
        Resource::VectorStore => validate_environment(auth, params_api_key.as_ref()),
    }
    .or_value_error()?;
    Ok(json!({
        "headers": headers,
        // without supplied params LiteLLM fills a params object nobody sees
        "params_api_key": params_api_key
            .as_ref()
            .filter(|_| step.params.is_some())
            .map(ExposeSecret::expose_secret),
    }))
}

fn urls_and_auth(resource: AzureResource, call: &UrlsAndAuth) -> HelperResult {
    let mut urls = Vec::with_capacity(call.urls.len());
    for step in &call.urls {
        urls.push(url_report(resource, step)?);
    }
    let mut auth = Vec::with_capacity(call.auth.len());
    for step in &call.auth {
        auth.push(auth_report(step)?);
    }
    Ok(json!({ "urls": urls, "auth": auth }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ContainerApiVersionQuery(steps) | Input::ContainerNormalizedBase(steps) => {
                let mut reports = Vec::with_capacity(steps.len());
                for step in steps {
                    reports.push(url_report(AzureResource::Container, step)?);
                }
                Ok(json!({ "steps": reports }))
            }
            Input::VideoSupportedFields(steps) => {
                let reports: Vec<_> = steps
                    .iter()
                    .map(|step| {
                        json!({
                            "supported": VIDEO_SUPPORTED_PARAMS,
                            "mapped": video_map_params(&step.options),
                        })
                    })
                    .collect();
                Ok(json!({ "steps": reports }))
            }
            Input::VideoAuthAndUrl(call) => urls_and_auth(AzureResource::Video, call),
            Input::VectorSearchAzureBase(call) => urls_and_auth(AzureResource::VectorStore, call),
        }
    }
}
