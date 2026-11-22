use std::collections::BTreeMap;

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
    let urls = url_steps(&runtime, &input.urls)?;
    let video = video_steps(&runtime, &input.video)?;
    Ok(json!({"urls": urls, "video": video}))
}

fn url_steps(runtime: &Runtime, steps: &[UrlStep]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut urls = Vec::with_capacity(steps.len());
    for step in steps {
        // the step's environment stands in for os.environ, which the Python helper sets for its call
        let fallback = step
            .module
            .api_base
            .clone()
            .or_else(|| step.environment.get("AZURE_API_BASE").cloned());
        let request = UrlRequest {
            api_base: step.api_base.as_deref(),
            fallback_api_base: fallback.as_deref(),
            api_version: step.api_version.as_deref(),
        };
        let Outcome::Url(url) = through(
            runtime,
            Call::Url {
                resource: AzureResource::Container,
                request,
            },
        )?
        else {
            return Err(failure(OTHER));
        };
        urls.push(json!({"url": url, "params_api_version": step.api_version}));
    }
    Ok(urls)
}

fn video_steps(runtime: &Runtime, steps: &[VideoStep]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut video = Vec::with_capacity(steps.len());
    for step in steps {
        let options: VideoCreate =
            serde_json::from_value(Value::Object(step.options.clone())).map_err(failure)?;
        let Outcome::VideoParams(mapped) = through(runtime, Call::VideoParams(&options))? else {
            return Err(failure(OTHER));
        };
        let Outcome::VideoSupported(supported) = through(runtime, Call::VideoSupported)? else {
            return Err(failure(OTHER));
        };
        video.push(json!({"supported": supported, "mapped": *mapped}));
    }
    Ok(video)
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

fn through(runtime: &Runtime, call: Call<'_>) -> Result<Outcome, Box<HelperError>> {
    let result = runtime.block_on(dispatch(AzureResourceOperation::ResourceUrls(Box::new(
        resource_urls::Dispatch { call },
    ))));
    match result {
        Ok(AzureResourceResult::ResourceUrls(outcome)) => Ok(outcome),
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::ResourceUrls(error)) => Err(failure(&error)),
        Err(error) => Err(failure(error)),
    }
}
