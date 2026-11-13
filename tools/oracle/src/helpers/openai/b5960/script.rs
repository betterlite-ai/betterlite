use std::sync::Mutex;

use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};

use crate::helpers::HelperError;

pub(super) const FIXED_NOW: i64 = 1_700_000_000;
pub(super) const FIXED_CREATED: u64 = 1_700_000_000;
pub(super) const COMPONENT_STATUS: u16 = 418;

pub(super) struct WordCounter;

impl TextTokenCounter for WordCounter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX)
    }
}

pub(super) fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| failure(&source.to_string()))
}

pub(super) fn fixture_headers() -> Headers {
    Headers::from([(String::from("x-fixture"), String::from("kept"))])
}

pub(super) fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("HelperError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
