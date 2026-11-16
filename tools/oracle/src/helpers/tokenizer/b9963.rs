use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_tokenizer::engine::tiktoken::Encoding;
use betterlite_tokenizer::image::settings::Settings as ImageSettings;
use betterlite_tokenizer::media::count::count_token_media;
use betterlite_tokenizer::media::settings::{ImageCounting, Settings as MediaSettings};
use betterlite_tokenizer::media::wire::TokenMedia;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::Builder;

use super::media_tokens::{self, Fixture};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    settings: ImageSettings,
    #[serde(default)]
    urls: BTreeMap<String, String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    CountContent {
        content: Vec<Value>,
        #[serde(default)]
        use_default_image_token_count: bool,
        #[serde(default)]
        default_token_count: Option<u64>,
    },
}

struct MappingFetcher<'urls> {
    urls: &'urls BTreeMap<String, String>,
}

impl RemoteMediaFetcher for MappingFetcher<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(
            self.urls
                .get(url)
                .cloned()
                .ok_or_else(|| ImageFetchError::Attempts {
                    url: url.to_owned(),
                }),
        )
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let mapped: Result<Vec<String>, ImageFetchError> = urls
            .iter()
            .map(|url| {
                self.urls
                    .get(url)
                    .cloned()
                    .ok_or_else(|| ImageFetchError::Attempts { url: url.clone() })
            })
            .collect();
        ready(mapped)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| media_tokens::failure(error.to_string()))?;
        runtime.block_on(run_calls(input))
    }
}

async fn run_calls(input: &Input) -> HelperResult {
    let fixture = media_tokens::fixture(Encoding::Cl100kBase)?;
    let fetcher = MappingFetcher { urls: &input.urls };
    let mut values = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        values.push(run_call(call, &fixture, &fetcher, &input.settings).await?);
    }
    Ok(Value::Array(values))
}

async fn run_call(
    call: &Call,
    fixture: &Fixture,
    fetcher: &MappingFetcher<'_>,
    image: &ImageSettings,
) -> HelperResult {
    let Call::CountContent {
        content,
        use_default_image_token_count,
        default_token_count,
    } = call;
    let content = content
        .iter()
        .map(to_raw_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| media_tokens::failure(error.to_string()))?;
    let media = TokenMedia { content };
    let settings = MediaSettings {
        image: *image,
        fallback_token_count: *default_token_count,
        image_counting: if *use_default_image_token_count {
            ImageCounting::DefaultCount
        } else {
            ImageCounting::Exact
        },
    };
    let counted = count_token_media(&fixture.texter(), &media, &settings, fetcher)
        .await
        .map_err(|error| media_tokens::failure(error.to_string()))?;
    Ok(json!(counted.tokens))
}
