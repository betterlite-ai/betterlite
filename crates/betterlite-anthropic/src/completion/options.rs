use betterlite_config::sections::anthropic_completion::OptionValues;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use crate::completion::error::CompletionError;
use crate::option_primitives::map_stop_sequences;

// extra_headers is advertised but map_openai_params never maps it
pub const SUPPORTED_OPENAI_PARAMS: [&str; 8] = [
    "stream",
    "max_tokens",
    "max_completion_tokens",
    "stop",
    "temperature",
    "top_p",
    "extra_headers",
    "user",
];

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct OptionDefaults {
    pub max_tokens_to_sample: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub metadata: Option<IndexMap<String, Json>>,
}

impl OptionDefaults {
    // the deployment's litellm_params win over litellm_settings, and the constructor's DEFAULT_MAX_TOKENS fills what both leave out
    #[must_use]
    pub fn resolve(
        deployment: &OptionValues,
        global: &OptionValues,
        default_max_tokens: i64,
    ) -> Self {
        Self {
            max_tokens_to_sample: deployment
                .max_tokens_to_sample
                .or(global.max_tokens_to_sample)
                .or(Some(default_max_tokens)),
            stop_sequences: deployment
                .stop_sequences
                .clone()
                .or_else(|| global.stop_sequences.clone()),
            temperature: deployment.temperature.or(global.temperature),
            top_p: deployment.top_p.or(global.top_p),
            top_k: deployment.top_k.or(global.top_k),
            metadata: deployment
                .metadata
                .clone()
                .or_else(|| global.metadata.clone()),
        }
    }
}

// the stop mapper reads the global litellm.drop_params, and the drop_params argument the method receives goes unused
pub fn map_openai_params(
    non_default_params: &IndexMap<String, Json>,
    mut optional_params: IndexMap<String, Json>,
    litellm_drop_params: bool,
) -> Result<IndexMap<String, Json>, CompletionError> {
    for (param, value) in non_default_params {
        match (param.as_str(), value) {
            ("max_tokens" | "max_completion_tokens", _) => {
                optional_params.insert("max_tokens_to_sample".to_owned(), value.clone());
            }
            ("stream", Json::Bool(true)) => {
                optional_params.insert("stream".to_owned(), Json::Bool(true));
            }
            ("stop", Json::Text(_) | Json::List(_)) => {
                let mapped = map_stop_sequences(value, litellm_drop_params)
                    .map_err(|error| CompletionError::Options(Box::new(error)))?;
                if let Some(stops) = mapped {
                    let texts = stops.into_iter().map(Json::Text).collect();
                    optional_params.insert("stop_sequences".to_owned(), Json::List(texts));
                }
            }
            ("temperature" | "top_p", _) => {
                optional_params.insert(param.clone(), value.clone());
            }
            ("user", _) => {
                let user = IndexMap::from([("user_id".to_owned(), value.clone())]);
                optional_params.insert("metadata".to_owned(), Json::Object(user));
            }
            _ => {}
        }
    }
    Ok(optional_params)
}
