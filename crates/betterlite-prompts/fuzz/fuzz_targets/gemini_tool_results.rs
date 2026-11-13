#![no_main]

use std::future::{Future, ready};

use betterlite_prompts::gemini_tool_results::convert::convert_tool_result_to_gemini;
use betterlite_prompts::gemini_tool_results::parser::parse_tool_result_message;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Builder;

struct RefusingFetch;

impl RemoteMediaFetcher for RefusingFetch {
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

fuzz_target!(|bytes: &[u8]| {
    let Ok(message) = parse_tool_result_message(bytes) else {
        return;
    };
    let Ok(runtime) = Builder::new_current_thread().build() else {
        return;
    };
    let fetch = RefusingFetch;
    drop(runtime.block_on(convert_tool_result_to_gemini(&message, None, true, &fetch)));
    drop(runtime.block_on(convert_tool_result_to_gemini(&message, None, false, &fetch)));
});
