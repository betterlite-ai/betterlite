use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_bedrock::bedrock_model_capabilities::ModelInfoLookup;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use indexmap::IndexMap;

use super::input::ModelCost;

pub(super) struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
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

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

pub(super) struct MapLookup(BTreeMap<String, IndexMap<String, Json>>);

impl MapLookup {
    pub(super) fn from_entries(entries: &ModelCost) -> Self {
        Self(
            entries
                .iter()
                .map(|(model, fields)| (model.clone(), fields.clone()))
                .collect(),
        )
    }
}

impl ModelInfoLookup for MapLookup {
    type Error = Unmapped;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Unmapped>> + Send {
        let answer = self
            .0
            .get(model)
            .or_else(|| self.0.get(&format!("bedrock/{model}")))
            .cloned();
        ready(answer.map(|fields| ModelInfo { fields }).ok_or(Unmapped))
    }
}

pub(super) struct NoFileBlocks;

impl FileBlocks for NoFileBlocks {
    type Failure = Infallible;

    fn file_block(&self, element: &Object) -> Result<Object, Infallible> {
        Ok(element.clone())
    }
}

pub(super) struct NoReasoning;

impl ReasoningTokenCounter for NoReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub(super) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct MapFetcher {
    urls: BTreeMap<String, String>,
    calls: Mutex<Vec<String>>,
}

impl MapFetcher {
    pub(super) fn new(entries: &BTreeMap<String, String>) -> Self {
        Self {
            urls: entries.clone(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RemoteMediaFetcher for MapFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        let answer = self
            .urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            });
        ready(answer)
    }

    async fn convert_urls(&self, urls: &[String]) -> Result<Vec<String>, ImageFetchError> {
        let mut fetched = Vec::with_capacity(urls.len());
        for url in urls {
            fetched.push(self.convert_url(url).await?);
        }
        Ok(fetched)
    }
}
