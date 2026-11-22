use betterlite_models::catalog::ModelCatalog;
use betterlite_models::metadata::identity::Mode;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest, ResolvedModel};

// get_llm_provider behind the entry: the caller injects the resolver with its catalog, provider sets and defaults
pub trait ProviderResolver: Send + Sync {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Result<ResolvedModel, ResolutionError>;
}

// LiteLLM's `model in litellm.model_cost` plus the entry's `mode` field, which may be absent
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogMode {
    NotListed,
    NoMode,
    Mode(String),
}

pub trait CatalogModes: Send + Sync {
    fn catalog_mode(&self, model: &str) -> CatalogMode;
}

impl CatalogModes for ModelCatalog {
    fn catalog_mode(&self, model: &str) -> CatalogMode {
        match self.metadata(model) {
            None => CatalogMode::NotListed,
            Some(metadata) => match metadata.mode {
                None => CatalogMode::NoMode,
                Some(mode) => CatalogMode::Mode(mode_name(mode).to_owned()),
            },
        }
    }
}

// the catalog's mode strings, which are exactly the serde names of the metadata Mode
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
