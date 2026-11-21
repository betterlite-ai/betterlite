use std::collections::BTreeMap;
use std::env;

use betterlite_azure::resource_urls::{AzureResource, UrlRequest};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::videos::create::VideoCreate;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_resource_dispatch::resource_urls::{self, Call, Outcome};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::support::{OTHER, failure, runtime};

// B547's container URL override and its delegated video parameter list, each from the leaf's own call
pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let mut urls = Vec::with_capacity(input.urls.len());
    for step in &input.urls {
        urls.push(url_view(&runtime, step)?);
    }
    let mut video = Vec::with_capacity(input.video.len());
    for step in &input.video {
        video.push(video_view(&runtime, step)?);
    }
    Ok(json!({"urls": urls, "video": video}))
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) urls: Vec<UrlStep>,
    #[serde(default)]
    pub(super) video: Vec<VideoStep>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlStep {
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_version: Option<String>,
    #[serde(default)]
    pub(super) module: Module,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Module {
    #[serde(default)]
    pub(super) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VideoStep {
    pub(super) options: Object,
}

fn url_view(runtime: &Runtime, step: &UrlStep) -> Result<Value, Box<HelperError>> {
    let from_env = env::var("AZURE_API_BASE").ok();
    let fallback = step.module.api_base.clone().or(from_env);
    let request = UrlRequest {
        api_base: step.api_base.as_deref(),
        fallback_api_base: fallback.as_deref(),
        api_version: step.api_version.as_deref(),
    };
    let result = runtime.block_on(dispatch(AzureResourceOperation::ResourceUrls(
        resource_urls::Dispatch {
            call: Call::Url {
                resource: AzureResource::Container,
                request,
            },
        },
    )));
    let url = match result {
        Ok(AzureResourceResult::ResourceUrls(Outcome::Url(url))) => url,
        Ok(_) => return Err(failure(OTHER)),
        Err(AzureResourceError::ResourceUrls(error)) => return Err(failure(&error)),
        Err(error) => return Err(failure(error)),
    };
    Ok(json!({"url": url, "params_api_version": step.api_version}))
}

fn video_view(runtime: &Runtime, step: &VideoStep) -> Result<Value, Box<HelperError>> {
    let options: VideoCreate =
        serde_json::from_value(Value::Object(step.options.clone())).map_err(failure)?;
    let result = runtime.block_on(dispatch(AzureResourceOperation::ResourceUrls(
        resource_urls::Dispatch {
            call: Call::VideoParams(&options),
        },
    )));
    let mapped = match result {
        Ok(AzureResourceResult::ResourceUrls(Outcome::VideoParams(mapped))) => *mapped,
        Ok(_) => return Err(failure(OTHER)),
        Err(AzureResourceError::ResourceUrls(error)) => return Err(failure(&error)),
        Err(error) => return Err(failure(error)),
    };
    let supported = match runtime.block_on(dispatch(AzureResourceOperation::ResourceUrls(
        resource_urls::Dispatch {
            call: Call::VideoSupported,
        },
    ))) {
        Ok(AzureResourceResult::ResourceUrls(Outcome::VideoSupported(supported))) => supported,
        Ok(_) => return Err(failure(OTHER)),
        Err(AzureResourceError::ResourceUrls(error)) => return Err(failure(&error)),
        Err(error) => return Err(failure(error)),
    };
    Ok(json!({"supported": supported, "mapped": mapped}))
}
