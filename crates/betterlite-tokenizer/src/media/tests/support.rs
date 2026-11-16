use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;

use crate::count::settings::Settings as CountSettings;
use crate::count::text::{Counted, Texter};
use crate::engine::Engine;
use crate::engine::assets;
use crate::engine::error::EngineError;
use crate::engine::huggingface::HuggingFaceEngine;
use crate::engine::limits::{BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService};
use crate::engine::tiktoken::{Encoding, TiktokenEngine};
use crate::image::settings::Settings as ImageSettings;
use crate::media::count::{self, Counted as MediaCounted};
use crate::media::error::MediaError;
use crate::media::settings::{ImageCounting, Settings};
use crate::media::wire::TokenMedia;

// every whitespace-separated word is one token, so a case's arithmetic stays readable
const WORDS: &str = r#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,"world":2,"hi":3},"unk_token":"[UNK]"}}"#;

struct DirectRunner;

#[async_trait]
impl BlockingCounter for DirectRunner {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

pub(crate) struct Fixture {
    service: Arc<CountingService>,
    counted: Counted,
    max_exact_chars: NonZeroUsize,
}

impl Fixture {
    pub(crate) fn texter(&self) -> Texter<'_> {
        Texter {
            counter: self.service.as_ref(),
            counted: &self.counted,
            max_exact_chars: self.max_exact_chars,
        }
    }
}

fn fixture_with(engine: Engine, chunks: Option<NonZeroUsize>) -> Fixture {
    let settings = CountSettings::default();
    let limits = settings.encode_limits().unwrap();
    Fixture {
        service: Arc::new(CountingService::new(4, Arc::new(DirectRunner), limits)),
        counted: Counted { engine, chunks },
        max_exact_chars: settings.max_exact_chars,
    }
}

pub(crate) fn words_fixture() -> Fixture {
    let engine = HuggingFaceEngine::from_bytes(WORDS.as_bytes()).unwrap();
    fixture_with(Engine::HuggingFace(Arc::new(engine)), None)
}

pub(crate) fn cl100k_fixture() -> Fixture {
    let settings = CountSettings::default();
    let data = assets::bundled(Encoding::Cl100kBase).unwrap();
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).unwrap();
    fixture_with(
        Engine::Tiktoken(Arc::new(engine)),
        Some(settings.chunk_chars),
    )
}

// a fetch would be a bug on the paths that must not fetch, so the refusal doubles as the guard
pub(crate) struct RefusingFetcher {
    pub(crate) calls: AtomicUsize,
}

impl RemoteMediaFetcher for RefusingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(ImageFetchError::Disabled {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.calls.fetch_add(urls.len(), Ordering::SeqCst);
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Disabled { url }))
    }
}

pub(crate) struct MappingFetcher {
    pub(crate) urls: BTreeMap<String, String>,
    pub(crate) calls: AtomicUsize,
}

impl RemoteMediaFetcher for MappingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
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
        self.calls.fetch_add(urls.len(), Ordering::SeqCst);
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

pub(crate) async fn count<Fetcher: RemoteMediaFetcher + Sync>(
    fixture: &Fixture,
    input: &str,
    settings: &Settings,
    fetcher: &Fetcher,
) -> Result<MediaCounted, MediaError> {
    let media = TokenMedia::try_from(input.as_bytes())?;
    count::count_token_media(&fixture.texter(), &media, settings, fetcher).await
}

pub(crate) fn plain(inner: &str) -> String {
    format!(r#"{{"content":[{inner}]}}"#)
}

pub(crate) fn image_url(url: &str, detail: &str) -> String {
    format!(
        r#"{{"content":[{{"type":"image_url","image_url":{{"url":"{url}","detail":"{detail}"}}}}]}}"#
    )
}

pub(crate) fn image_url_string(url: &str) -> String {
    format!(r#"{{"content":[{{"type":"image_url","image_url":"{url}"}}]}}"#)
}

pub(crate) fn document(source: &str, title: &str) -> String {
    format!(r#"{{"content":[{{"type":"document","title":"{title}","source":{source}}}]}}"#)
}

pub(crate) fn file(filename: &str, file_data: &str) -> String {
    format!(
        r#"{{"content":[{{"type":"file","file":{{"filename":"{filename}","file_data":"{file_data}"}}}}]}}"#
    )
}

pub(crate) fn nested_documents(depth: usize) -> String {
    let mut block = r#"{"type":"text","text":"hello"}"#.to_owned();
    for _ in 0..depth {
        block =
            format!(r#"{{"type":"document","source":{{"type":"content","content":[{block}]}}}}"#);
    }
    format!(r#"{{"content":[{block}]}}"#)
}

pub(crate) struct CaseOptions {
    pub(crate) counting: ImageCounting,
    pub(crate) fallback: Option<u64>,
    pub(crate) image_default_count: u64,
}

pub(crate) fn settings(options: &CaseOptions) -> Settings {
    Settings {
        image: ImageSettings {
            default_width: 300,
            default_height: 300,
            default_token_count: options.image_default_count,
            max_short_side: 768,
            max_long_side: 2000,
            tile_width: 512,
            tile_height: 512,
        },
        fallback_token_count: options.fallback,
        image_counting: options.counting,
    }
}
