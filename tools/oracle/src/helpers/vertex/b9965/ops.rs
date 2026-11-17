use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_request::body::{BodyInput, transform};
use betterlite_vertex::gemini_request::media_prepare::{FetchMode, prepare};
use betterlite_vertex::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    provider: ProviderName,
    mode: ModeName,
    messages: Vec<Message>,
    data_urls: BTreeMap<String, String>,
    optional_params: Object,
    litellm_params: Object,
    // the case states what the model catalog answers, so the comparison fixes the capability input
    supports: Supports,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Supports {
    system_message: bool,
    response_schema: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderName {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

impl ProviderName {
    const fn capability_provider(&self) -> VertexCapabilityProvider {
        match self {
            Self::VertexAi => VertexCapabilityProvider::VertexAi,
            Self::VertexAiBeta => VertexCapabilityProvider::VertexAiBeta,
            Self::Gemini => VertexCapabilityProvider::Gemini,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModeName {
    Sequential,
    Concurrent,
}

impl ModeName {
    const fn fetch_mode(&self) -> FetchMode {
        match self {
            Self::Sequential => FetchMode::Sequential,
            Self::Concurrent => FetchMode::Concurrent,
        }
    }
}

impl ModelCapabilityLookup for Input {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        Ok(match kind {
            CapabilityKind::SystemMessages => self.supports.system_message,
            CapabilityKind::ResponseSchema => self.supports.response_schema,
        })
    }
}

// the case's url map stands in for the network: a url it omits is a download that fails
struct MapFetch {
    data_urls: BTreeMap<String, String>,
}

impl MapFetch {
    fn lookup(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls.get(url).map_or_else(
            || {
                Err(ImageFetchError::Attempts {
                    url: url.to_owned(),
                })
            },
            |data_url| Ok(data_url.clone()),
        )
    }
}

impl RemoteMediaFetcher for MapFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.lookup(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.iter().map(|url| self.lookup(url)).collect())
    }
}

// conversion runs after prepare, so every fetch it still asks for is a case the map failed to inline
struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread().build()?;
    let provider = input.provider.capability_provider();
    let messages = runtime
        .block_on(prepare(
            &input.messages,
            provider,
            &MapFetch {
                data_urls: input.data_urls.clone(),
            },
            input.mode.fetch_mode(),
        ))
        .map_err(|error| bad_request("ImageFetchError", &error.to_string()))?;
    let body_input = BodyInput {
        provider,
        model: &input.model,
        messages: &messages,
        optional_params: &input.optional_params,
        litellm_params: &input.litellm_params,
        cached_content: None,
        modify_params: false,
        schema_template: None,
    };
    match runtime.block_on(transform(&body_input, input, &NoFetch)) {
        Ok(transformed) => Ok(json!({
            "body": transformed.body,
            "litellm_params": transformed.litellm_params,
        })),
        Err(error) => Err(bad_request("BadRequestError", &error.to_string())),
    }
}

// LiteLLM's download failures are BadRequestError subclasses and its conversion failures are BadRequestError; neither names a type, param or code
fn bad_request(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: Some(400),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    })
}
