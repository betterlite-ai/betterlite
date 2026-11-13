use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_context::bedrock_model::regions::AmazonBedrockGlobalConfig;
use betterlite_context::bedrock_model::strip_bedrock_routing_prefix;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::{BedrockNames, Resolved, find_info};
use betterlite_models::metadata::identity::Mode;

// What a health-check probe asks the model cost map about one model.
pub trait HealthCheckModelInfo {
    fn mode(&self, model: &str) -> Option<String>;

    fn supports_reasoning(&self, model: &str) -> bool;
}

pub struct CatalogModelInfo<'catalog> {
    pub catalog: &'catalog ModelCatalog,
}

// B2203's own strip lives in betterlite-bedrock, which this crate does not depend on
fn strip_mantle_region(model: &str) -> String {
    let Some((head, tail)) = model.split_once('/') else {
        return model.to_owned();
    };
    if AmazonBedrockGlobalConfig.get_all_regions().contains(&head) {
        tail.to_owned()
    } else {
        model.to_owned()
    }
}

const BEDROCK_NAMES: BedrockNames = BedrockNames {
    base_model: get_bedrock_base_model,
    strip_routing_prefix: strip_bedrock_routing_prefix,
    strip_mantle_region: strip_mantle_region,
};

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

impl HealthCheckModelInfo for CatalogModelInfo<'_> {
    fn mode(&self, model: &str) -> Option<String> {
        resolved(self.catalog, model)?
            .mode
            .map(mode_name)
            .map(str::to_owned)
    }

    fn supports_reasoning(&self, model: &str) -> bool {
        resolved(self.catalog, model).and_then(|info| info.capabilities.supports_reasoning)
            == Some(true)
    }
}
