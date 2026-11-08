#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use crate::arguments::json::{Json, Object};
use crate::media::inline::RemoteMediaFetcher;
use crate::media_fetch::error::ImageFetchError;

mod cache_control;
mod documents;
mod invoke;

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

fn maps(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(url, data_url)| ((*url).to_owned(), (*data_url).to_owned()))
        .collect()
}

fn object(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

fn list(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}
