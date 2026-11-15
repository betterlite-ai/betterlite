use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::provider_info::provider_info;
use betterlite_models::lookup::resolve::{Resolved, find_info, split_model};
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

// _get_model_cost_key: the exact key first, then the one that matches without case
fn bare_supports_reasoning(catalog: &ModelCatalog, model: &str) -> bool {
    let lowercase = model.to_lowercase();
    let canonical = catalog.lookup.get(model).or_else(|| {
        catalog
            .lookup
            .iter()
            .find(|(key, _)| key.to_lowercase() == lowercase)
            .map(|(_, entry_key)| entry_key)
    });
    canonical
        .and_then(|entry_key| catalog.entries.get(entry_key.as_ref()))
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

    // _supports_factory: get_llm_provider splits the route, then the entry, the bare entry and the provider info answer (#20885)
    fn supports_reasoning(&self, model: &str) -> bool {
        let split = split_model(self.catalog, model);
        let provider = split.provider.as_deref();
        let query = ModelQuery {
            model: &split.model,
            provider,
        };
        let Some(info) = find_info(self.catalog, &BEDROCK_NAMES, &query)
            .ok()
            .flatten()
        else {
            return provider_info(self.catalog, &split.model, provider).supports_reasoning
                == Some(true);
        };
        if let Some(flag) = info.capabilities.supports_reasoning {
            return flag;
        }
        bare_supports_reasoning(self.catalog, &split.model)
    }
}
