#![no_main]

use std::future::{Future, ready};

use betterlite_prompts::anthropic::assembly::{PromptDeps, anthropic_messages_pt};
use betterlite_prompts::anthropic::input::parse_prompt_anthropic_input;
use betterlite_prompts::anthropic_invokes::documents::FileMessageError;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Builder;

const GENERATED_ID: &str = "generated-id";

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

struct NoFileBlocks;

impl FileBlocks for NoFileBlocks {
    type Failure = FileMessageError;

    fn file_block(&self, _element: &Object) -> Result<Object, FileMessageError> {
        Err(FileMessageError::MissingFile)
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(input) = parse_prompt_anthropic_input(bytes) else {
        return;
    };
    let Ok(runtime) = Builder::new_current_thread().build() else {
        return;
    };
    let fetcher = RefusingFetch;
    let blocks = NoFileBlocks;
    let new_id = || GENERATED_ID.to_owned();
    let deps = PromptDeps {
        fetcher: &fetcher,
        blocks: &blocks,
        new_id: &new_id,
    };
    drop(runtime.block_on(anthropic_messages_pt(
        &input.messages,
        &input.model,
        &input.llm_provider,
        input.modify_params,
        &deps,
    )));
});
