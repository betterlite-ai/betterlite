use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::{Resolved, find_info};
use betterlite_models::metadata::identity::Mode;
use betterlite_tokenizer::model_capabilities::BEDROCK_NAMES;

pub trait HealthCheckModelInfo {
    fn mode(&self, model: &str) -> Option<String>;

    fn supports_reasoning(&self, model: &str) -> bool;
}

pub struct CatalogModelInfo<'catalog> {
    pub catalog: &'catalog ModelCatalog,
}

const fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::AudioSpeech => "audio_speech",
        Mode::AudioTranscription => "audio_transcription",
        Mode::Chat => "chat",
        Mode::Completion => "completion",
        Mode::Embedding => "embedding",
        Mode::Evaluation => "evaluation",
        Mode::Guardrail => "guardrail",
        Mode::ImageEdit => "image_edit",
        Mode::ImageGeneration => "image_generation",
        Mode::Moderation => "moderation",
        Mode::Ocr => "ocr",
        Mode::Realtime => "realtime",
        Mode::Rerank => "rerank",
        Mode::Responses => "responses",
        Mode::Search => "search",
        Mode::VectorStore => "vector_store",
        Mode::VideoGeneration => "video_generation",
    }
}

// get_model_info, with a failure reading the same as an unmapped model for the probe
fn resolved(catalog: &ModelCatalog, model: &str) -> Option<Resolved> {
    find_info(
        catalog,
        &BEDROCK_NAMES,
        &ModelQuery {
            model,
            provider: None,
        },
    )
    .ok()
    .flatten()
}

// _supports_factory: an entry that leaves the flag absent falls back to the bare model-name entry (#20885)
fn bare_supports_reasoning(catalog: &ModelCatalog, key: &str) -> bool {
    let bare = key.rsplit('/').next().unwrap_or(key);
    catalog
        .lookup
        .get(bare)
        .and_then(|canonical| catalog.entries.get(canonical.as_ref()))
        .and_then(|entry| entry.fields.capabilities.reasoning.supports_reasoning)
        == Some(true)
}

impl HealthCheckModelInfo for CatalogModelInfo<'_> {
    fn mode(&self, model: &str) -> Option<String> {
        resolved(self.catalog, model)?
            .mode
            .map(mode_name)
            .map(str::to_owned)
    }

    fn supports_reasoning(&self, model: &str) -> bool {
        let Some(info) = resolved(self.catalog, model) else {
            return false;
        };
        match info.capabilities.supports_reasoning {
            Some(flag) => flag,
            None => bare_supports_reasoning(self.catalog, &info.key),
        }
    }
}
