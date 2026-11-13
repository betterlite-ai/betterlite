use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::gemini_tool_results::convert::convert_tool_result_to_gemini;
use betterlite_prompts::gemini_tool_results::types::LastToolCallMessage;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::to_value;
use tokio::runtime::Builder;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    message: Message,
    #[serde(default)]
    last_message_with_tool_calls: Option<LastToolCallMessage>,
    #[serde(default)]
    forward_function_call_id: bool,
    #[serde(default)]
    data_urls: BTreeMap<String, String>,
}

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

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let source = MapSource {
            data_urls: &input.data_urls,
        };
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| helper_error("RuntimeError", error.to_string()))?;
        runtime
            .block_on(convert_tool_result_to_gemini(
                &input.message,
                input.last_message_with_tool_calls.as_ref(),
                input.forward_function_call_id,
                &source,
            ))
            .map_err(|error| helper_error("GeminiToolResultsError", error.to_string()))
            .and_then(ok_value)
    }
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    match to_value(payload) {
        Ok(value) => Ok(value),
        Err(error) => Err(helper_error("SerializationError", error.to_string())),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
