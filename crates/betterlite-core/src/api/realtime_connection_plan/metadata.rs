use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::find_info;
use betterlite_models::metadata::identity::Mode;
use betterlite_tokenizer::model_capabilities::BEDROCK_NAMES;
use thiserror::Error;

// the two fields _is_transcription_only_realtime_model reads out of get_model_info
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeModelInfo {
    pub mode: Option<Mode>,
    pub supported_endpoints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ModelInfoFailure {
    pub message: String,
}

// B2225's get_model_info, narrowed to what the health probe reads
pub trait RealtimeModelMetadata: Send + Sync {
    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<Option<RealtimeModelInfo>, ModelInfoFailure>;
}

impl RealtimeModelMetadata for ModelCatalog {
    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<Option<RealtimeModelInfo>, ModelInfoFailure> {
        let query = ModelQuery {
            model,
            provider: Some(custom_llm_provider),
        };
        let resolved =
            find_info(self, &BEDROCK_NAMES, &query).map_err(|error| ModelInfoFailure {
                message: error.to_string(),
            })?;
        Ok(resolved.map(|resolved| {
            let supported_endpoints = self
                .entries
                .get(&resolved.key)
                .map_or_else(Vec::new, |entry| {
                    entry.fields.metadata.supported_endpoints.clone()
                });
            RealtimeModelInfo {
                mode: resolved.mode,
                supported_endpoints,
            }
        }))
    }
}
