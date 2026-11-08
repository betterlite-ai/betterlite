use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;

pub(crate) struct UrlSource<'urls> {
    pub(crate) data_urls: &'urls BTreeMap<String, String>,
}

impl RemoteMediaFetcher for UrlSource<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(
            self.data_urls
                .get(url)
                .cloned()
                .ok_or_else(|| ImageFetchError::Attempts {
                    url: url.to_owned(),
                }),
        )
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(
            urls.iter()
                .map(|url| {
                    self.data_urls
                        .get(url)
                        .cloned()
                        .ok_or_else(|| ImageFetchError::Attempts { url: url.clone() })
                })
                .collect(),
        )
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file blocks")]
pub(crate) struct NoFileBlocks;

impl FileBlocks for NoFileBlocks {
    type Failure = Self;

    fn file_block(&self, _element: &Object) -> Result<Object, Self> {
        Err(Self)
    }
}
