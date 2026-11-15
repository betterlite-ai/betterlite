use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_provider::snowflake::response::{PostCall, PostCallLogger};
use serde_json::{Value, json};
use uuid::Uuid;

// The cases carry no remote media: LiteLLM would download it, and the oracle reads no network
pub(crate) struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.first().map_or_else(
            || Ok(Vec::new()),
            |url| Err(ImageFetchError::Rejected { url: url.clone() }),
        ))
    }
}

pub(crate) struct WordCounter;

impl ReasoningTokenCounter for WordCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Infallible> {
        Ok(text.split_whitespace().count())
    }
}

pub(crate) struct NilIds;

impl IdSource for NilIds {
    fn next_id(&self) -> Uuid {
        Uuid::nil()
    }
}

#[derive(Default)]
pub(crate) struct Recorder {
    calls: Mutex<Vec<Value>>,
}

impl Recorder {
    pub(crate) fn into_calls(self) -> Vec<Value> {
        self.calls
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl PostCallLogger for Recorder {
    fn post_call(&self, call: PostCall<'_>) {
        let entry = json!({
            "input": call.input,
            "api_key": call.api_key,
            "original_response": call.original_response,
            "complete_input_dict": call.complete_input_dict,
        });
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}
