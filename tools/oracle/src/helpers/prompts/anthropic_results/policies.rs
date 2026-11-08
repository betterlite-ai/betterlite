use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;

use super::Alphabet;

pub(super) struct CaseSource<'urls> {
    pub(super) data_urls: &'urls BTreeMap<String, String>,
}

impl CaseSource<'_> {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for CaseSource<'_> {
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

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file content blocks")]
pub(super) struct NoFileBlock;

pub(super) struct NoBlocks;

impl FileBlocks for NoBlocks {
    type Failure = NoFileBlock;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileBlock> {
        Err(NoFileBlock)
    }
}

pub(super) fn predicate(alphabet: Alphabet) -> fn(char) -> bool {
    match alphabet {
        Alphabet::Anthropic => {
            |candidate| !(candidate.is_ascii_alphanumeric() || candidate == '_' || candidate == '-')
        }
        Alphabet::Bedrock => |candidate| {
            !(candidate.is_ascii_alphanumeric() || matches!(candidate, '_' | '.' | ':' | '-'))
        },
    }
}
