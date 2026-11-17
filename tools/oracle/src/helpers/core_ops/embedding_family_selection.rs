use betterlite_core::embedding_rerank_providers::{embedding_config, rerank_config};
use betterlite_models::identity::provider_id::ProviderId;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

// the four providers whose arms this task owns; the factories' unrelated branches stay with theirs
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    provider: ProviderId,
    model: String,
    api_base: Option<String>,
    #[serde(default)]
    present_version_params: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let embedding =
                embedding_config(call.provider, &call.model).map(|config| config.to_string());
            let rerank = rerank_config(
                call.provider,
                call.api_base.as_deref(),
                &call.present_version_params,
            )
            .map(|config| config.to_string());
            answers.push(json!({
                "embedding": embedding,
                "rerank": rerank,
            }));
        }
        Ok(json!(answers))
    }
}
