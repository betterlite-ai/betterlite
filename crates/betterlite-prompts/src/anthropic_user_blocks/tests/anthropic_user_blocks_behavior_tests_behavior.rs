use std::collections::BTreeMap;
use std::future::{Future, ready};

use crate::anthropic_results::tool_result::FileBlocks;
use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::media_fetch::error::ImageFetchError;

mod prelude;
mod system_prompt;
mod user_run;

struct UrlMap {
    data_urls: BTreeMap<String, String>,
}

impl UrlMap {
    fn fetch(&self, url: &str) -> Result<String, ImageFetchError> {
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
        ready(self.fetch(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.iter().map(|url| self.fetch(url)).collect())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file blocks")]
struct NoFileBlocks;

impl FileBlocks for NoFileBlocks {
    type Failure = Self;

    fn file_block(&self, _element: &Object) -> Result<Object, Self> {
        Err(Self)
    }
}

struct RecordingBlocks;

impl FileBlocks for RecordingBlocks {
    type Failure = NoFileBlocks;

    fn file_block(&self, element: &Object) -> Result<Object, NoFileBlocks> {
        Ok(Object::from_iter([(
            "file_block".to_owned(),
            Json::Object(element.clone()),
        )]))
    }
}

fn messages(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}

fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

fn url_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(url, data_url)| ((*url).to_owned(), (*data_url).to_owned()))
        .collect()
}
