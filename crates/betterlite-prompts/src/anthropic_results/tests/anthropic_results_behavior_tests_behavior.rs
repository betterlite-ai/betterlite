#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use crate::anthropic_results::tool_result::FileBlocks;
use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::media_fetch::error::ImageFetchError;
use crate::message_content::Message;

mod ids_and_documents;
mod images;
mod invokes_and_results;
mod tool_result;

#[derive(Debug, thiserror::Error)]
#[error("the file block policy refused the element")]
struct NoFileBlocks;

struct MapSource<'urls> {
    data_urls: &'urls BTreeMap<String, String>,
}

impl MapSource<'_> {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for MapSource<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.result_for(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.iter().map(|url| self.result_for(url)).collect())
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

struct FailingBlocks;

impl FileBlocks for FailingBlocks {
    type Failure = NoFileBlocks;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileBlocks> {
        Err(NoFileBlocks)
    }
}

fn maps(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(url, data_url)| ((*url).to_owned(), (*data_url).to_owned()))
        .collect()
}

fn message(raw: &str) -> Message {
    serde_json::from_str(raw).unwrap()
}

fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}
