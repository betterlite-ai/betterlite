#![no_main]

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_tokenizer::count::settings::Settings as CountSettings;
use betterlite_tokenizer::count::text::{Counted, Texter};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::huggingface::HuggingFaceEngine;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService,
};
use betterlite_tokenizer::image::settings::Settings as ImageSettings;
use betterlite_tokenizer::media::count::count_token_media;
use betterlite_tokenizer::media::settings::{ImageCounting, Settings};
use betterlite_tokenizer::media::wire::TokenMedia;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Builder;

// every whitespace-separated word is one token, so the fixture stays cheap to build per input
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

fn settings(mode: ImageCounting) -> Settings {
    Settings {
        image: ImageSettings::default(),
        fallback_token_count: None,
        image_counting: mode,
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(media) = TokenMedia::try_from(bytes) else {
        return;
    };
    let count_settings = CountSettings::default();
    let Ok(limits) = count_settings.encode_limits() else {
        return;
    };
    let Ok(engine) = HuggingFaceEngine::from_bytes(WORDS.as_bytes()) else {
        return;
    };
    let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
        return;
    };
    let service = CountingService::new(4, Arc::new(DirectRunner), limits);
    let counted = Counted {
        engine: Engine::HuggingFace(Arc::new(engine)),
        chunks: None,
    };
    let texter = Texter {
        counter: &service,
        counted: &counted,
        max_exact_chars: count_settings.max_exact_chars,
    };
    for mode in [ImageCounting::Exact, ImageCounting::DefaultCount] {
        drop(runtime.block_on(count_token_media(&texter, &media, &settings(mode))));
    }
});
