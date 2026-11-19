use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::{Future, ready};

use betterlite_bedrock::claude_platform::WorkspaceSources;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::claude_platform_inputs::{PlatformEnvironment, PlatformInputs};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use serde_json::{Value, json};

pub(super) fn invalid<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// The cases name no huggingface model, so the transport refuses instead of reaching the network.
pub(super) struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the case must not fetch a huggingface config")]
pub(super) struct NoHfError;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}

pub(super) struct RefusingFetcher;

fn refused(url: &str) -> ImageFetchError {
    ImageFetchError::Disabled {
        url: url.to_owned(),
    }
}

impl RemoteMediaFetcher for RefusingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(refused(url)))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(refused(urls.first().map_or("", String::as_str))))
    }
}

pub(super) struct NoFileBlocks;

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file blocks")]
pub(super) struct NoFileError;

impl FileBlocks for NoFileBlocks {
    type Failure = NoFileError;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileError> {
        Err(NoFileError)
    }
}

// The case values parse through the B2211 loader, so the workspace fields arrive the way a config file hands them over.
pub(super) fn platform_inputs(
    deployment: &BTreeMap<String, String>,
    litellm_settings: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
) -> Result<PlatformInputsCase, Box<HelperError>> {
    let mut params = serde_json::Map::new();
    params.insert("model".to_owned(), Value::String("bedrock/unit".to_owned()));
    for (key, value) in deployment {
        params.insert(key.clone(), Value::String(value.clone()));
    }
    let document = json!({
        "model_list": [{"model_name": "unit", "litellm_params": params}],
        "litellm_settings": litellm_settings,
    });
    let source = serde_saphyr::to_string(&document).map_err(invalid)?;
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source)
    .map_err(invalid)?;
    let section = config.sections.claude_platform_inputs;
    Ok(PlatformInputsCase {
        deployment: section.deployments.first().cloned().unwrap_or_default(),
        global: section.litellm_settings,
        environment: section.environment,
    })
}

pub(super) struct PlatformInputsCase {
    pub(super) deployment: PlatformInputs,
    pub(super) global: PlatformInputs,
    pub(super) environment: PlatformEnvironment,
}

impl PlatformInputsCase {
    pub(super) const fn sources(&self) -> WorkspaceSources<'_> {
        WorkspaceSources {
            deployment: &self.deployment,
            global: &self.global,
            environment: &self.environment,
        }
    }
}
