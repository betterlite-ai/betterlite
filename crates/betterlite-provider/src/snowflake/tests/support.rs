use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_models::decisions::Json as AnswerJson;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct MediaSource {
    pub(crate) conversions: Mutex<Vec<String>>,
    pub(crate) mapping: BTreeMap<String, String>,
    pub(crate) slow: bool,
}

impl MediaSource {
    #[must_use]
    pub(crate) fn with(pairs: &[(&str, &str)]) -> Self {
        Self {
            conversions: Mutex::new(Vec::new()),
            mapping: pairs
                .iter()
                .map(|(url, data_url)| ((*url).to_owned(), (*data_url).to_owned()))
                .collect(),
            slow: false,
        }
    }

    // its fetches are not ready on the first poll, like a real download
    #[must_use]
    pub(crate) fn slow(pairs: &[(&str, &str)]) -> Self {
        Self {
            slow: true,
            ..Self::with(pairs)
        }
    }

    pub(crate) fn converted(&self) -> Vec<String> {
        self.conversions.lock().unwrap().clone()
    }
}

impl RemoteMediaFetcher for MediaSource {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.conversions.lock().unwrap().push(url.to_owned());
        let outcome = self.result_for(url);
        let slow = self.slow;
        async move {
            if slow {
                tokio::task::yield_now().await;
            }
            outcome
        }
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.conversions
            .lock()
            .unwrap()
            .extend(urls.iter().cloned());
        let results = urls.iter().map(|url| self.result_for(url)).collect();
        ready(results)
    }
}

impl MediaSource {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.mapping
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Rejected {
                url: url.to_owned(),
            })
    }
}

pub(crate) struct WordCounter;

impl ReasoningTokenCounter for WordCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Infallible> {
        Ok(text.split_whitespace().count())
    }
}

pub(crate) struct SequenceIds {
    next: AtomicU64,
}

impl Default for SequenceIds {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(0),
        }
    }
}

impl IdSource for SequenceIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.next.fetch_add(1, Ordering::SeqCst)))
    }
}

pub(crate) fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn answer(source: &str) -> AnswerJson {
    serde_json::from_str(source).unwrap()
}

pub(crate) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}
