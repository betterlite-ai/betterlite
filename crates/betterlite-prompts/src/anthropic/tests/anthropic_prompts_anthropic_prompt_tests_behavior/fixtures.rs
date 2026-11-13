use std::collections::BTreeMap;
use std::future::{Future, ready};

use crate::anthropic::assembly::{PromptDeps, anthropic_messages_pt};
use crate::anthropic::error::AnthropicPromptError;
use crate::anthropic_results::tool_result::FileBlocks;
use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::media_fetch::error::ImageFetchError;

pub(super) const MODEL: &str = "claude-sonnet-4-5";
pub(super) const PROVIDER: &str = "anthropic";
pub(super) const GENERATED_ID: &str = "generated-id";

pub(super) type TestResult = Result<Vec<Json>, AnthropicPromptError<NoFileBlocksError>>;

#[derive(Debug, thiserror::Error)]
#[error("the test supplies no file blocks")]
pub(super) struct NoFileBlocksError;

pub(super) struct NoFileBlocks;

impl FileBlocks for NoFileBlocks {
    type Failure = NoFileBlocksError;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileBlocksError> {
        Err(NoFileBlocksError)
    }
}

pub(super) struct UrlMap {
    pub(super) data_urls: BTreeMap<String, String>,
}

impl UrlMap {
    fn resolve(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for UrlMap {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.resolve(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.iter().map(|url| self.resolve(url)).collect())
    }
}

pub(super) fn messages(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

pub(super) async fn convert(raw: &str, modify_params: bool) -> TestResult {
    convert_values(&messages(raw), MODEL, modify_params, &[]).await
}

pub(super) async fn convert_values(
    values: &[Json],
    model: &str,
    modify_params: bool,
    data_urls: &[(&str, &str)],
) -> TestResult {
    let fetcher = UrlMap {
        data_urls: data_urls
            .iter()
            .map(|(url, data)| ((*url).to_owned(), (*data).to_owned()))
            .collect(),
    };
    let blocks = NoFileBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let deps = PromptDeps {
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    anthropic_messages_pt(values, model, PROVIDER, modify_params, &deps).await
}

pub(super) async fn converted(raw: &str, modify_params: bool) -> Vec<Json> {
    convert(raw, modify_params)
        .await
        .expect("the conversion must succeed")
}

pub(super) async fn failure(
    raw: &str,
    modify_params: bool,
) -> AnthropicPromptError<NoFileBlocksError> {
    convert(raw, modify_params)
        .await
        .expect_err("the conversion must fail")
}

pub(super) fn expected(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}
