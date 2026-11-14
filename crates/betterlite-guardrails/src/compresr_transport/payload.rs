use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use super::{CompressTarget, TransportSettings};

const SOURCE_TAG: &str = "integration:litellm";
const NAMED_KEYS: [&str; 5] = [
    "compression_model_name",
    "target_compression_ratio",
    "coarse",
    "dynamic",
    "source",
];

#[derive(Debug, Serialize)]
pub(super) struct CompressRequest {
    pub(super) url: String,
    pub(super) payload: CompressPayload,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum CompressPayload {
    Single(SinglePayload),
    Batch(BatchPayload),
}

#[derive(Debug, Serialize)]
pub(super) struct SinglePayload {
    context: String,
    query: String,
    #[serde(flatten)]
    common: CommonParams,
}

#[derive(Debug, Serialize)]
pub(super) struct BatchPayload {
    inputs: Vec<CompressInput>,
    #[serde(flatten)]
    common: CommonParams,
}

#[derive(Debug, Serialize)]
pub(super) struct CompressInput {
    context: String,
    query: String,
}

#[derive(Debug, Serialize)]
pub(super) struct CommonParams {
    compression_model_name: String,
    target_compression_ratio: f64,
    coarse: bool,
    dynamic: bool,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dynamic_min_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dynamic_max_ratio: Option<f64>,
    #[serde(flatten)]
    extra: BTreeMap<String, GuardrailValue>,
}

pub(super) fn build(settings: &TransportSettings, targets: &[CompressTarget]) -> CompressRequest {
    let common = common_params(settings);
    if let [target] = targets {
        return CompressRequest {
            url: format!("{}/api/compress/question-specific/", settings.api_base),
            payload: CompressPayload::Single(SinglePayload {
                context: target.context.clone(),
                query: target.query.clone(),
                common,
            }),
        };
    }
    CompressRequest {
        url: format!("{}/api/compress/question-specific/batch", settings.api_base),
        payload: CompressPayload::Batch(BatchPayload {
            inputs: targets
                .iter()
                .map(|target| CompressInput {
                    context: target.context.clone(),
                    query: target.query.clone(),
                })
                .collect(),
            common,
        }),
    }
}

fn common_params(settings: &TransportSettings) -> CommonParams {
    CommonParams {
        compression_model_name: settings.model.clone(),
        target_compression_ratio: settings.target_compression_ratio,
        coarse: settings.coarse,
        dynamic: settings.dynamic,
        source: SOURCE_TAG,
        dynamic_min_ratio: settings.dynamic_min_ratio,
        dynamic_max_ratio: settings.dynamic_max_ratio,
        extra: passthrough(settings),
    }
}

// LiteLLM spreads the passthrough first, so a key it also names is replaced by the named value
fn passthrough(settings: &TransportSettings) -> BTreeMap<String, GuardrailValue> {
    settings
        .compression_params
        .iter()
        .filter(|(key, _)| {
            !NAMED_KEYS.contains(&key.as_str())
                && !(key.as_str() == "dynamic_min_ratio" && settings.dynamic_min_ratio.is_some())
                && !(key.as_str() == "dynamic_max_ratio" && settings.dynamic_max_ratio.is_some())
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
