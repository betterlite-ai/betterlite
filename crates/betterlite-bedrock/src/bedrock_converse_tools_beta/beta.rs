use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::feature_flags::FeatureFlagsError;
use betterlite_anthropic::feature_flags::beta::EFFORT_BETA;
use betterlite_anthropic::feature_flags::tools::is_eager_input_streaming_used;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::Json as PromptsJson;
use indexmap::IndexMap;

use crate::bedrock_converse_tools_beta::context_management::filter_context_management;
use crate::bedrock_messages_beta::beta::FINE_GRAINED_STREAMING_BETA;
use crate::event_frames::beta::{BetaHeader, BetaValue, anthropic_beta_from_headers};
use crate::message_options::{ADAPTIVE_THINKING, BEDROCK_PROVIDER};

pub const ANTHROPIC_BETA: &str = "anthropic_beta";
const OUTPUT_CONFIG: &str = "output_config";
const EFFORT: &str = "effort";

#[must_use]
pub fn client_betas(headers: Option<&BTreeMap<String, BetaHeader>>) -> Vec<Json> {
    let values = headers.map_or_else(Vec::new, anthropic_beta_from_headers);
    values.into_iter().map(beta_value).collect()
}

// LiteLLM adds the eager beta only behind the anthropic gate, so a foreign model never parses the flag.
pub fn initial_betas(
    headers: Option<&BTreeMap<String, BetaHeader>>,
    prompts: &[PromptsJson],
    anthropic_base: bool,
) -> Result<Vec<Json>, FeatureFlagsError> {
    let mut betas = client_betas(headers);
    if anthropic_base
        && is_eager_input_streaming_used(prompts)?
        && !has_beta(&betas, FINE_GRAINED_STREAMING_BETA)
    {
        betas.push(Json::Text(FINE_GRAINED_STREAMING_BETA.to_owned()));
    }
    Ok(betas)
}

// Effort, the context-management filter and the request field are all gated on the anthropic base.
pub async fn apply_anthropic_betas<Source, Transport>(
    params: &mut IndexMap<String, Json>,
    model: &str,
    betas: &mut Vec<Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    if let Some(Json::Object(output_config)) = params.get(OUTPUT_CONFIG)
        && output_config
            .get(EFFORT)
            .is_some_and(|effort| !matches!(effort, Json::Null))
        && !supports_model_capability(
            model,
            ADAPTIVE_THINKING,
            BEDROCK_PROVIDER,
            environment,
            catalogs,
        )
        .await
        && !has_beta(betas, EFFORT_BETA)
    {
        betas.push(Json::Text(EFFORT_BETA.to_owned()));
    }
    filter_context_management(params, betas);
    if !betas.is_empty() {
        params.insert(ANTHROPIC_BETA.to_owned(), Json::List(betas.clone()));
    }
}

#[must_use]
pub fn has_beta(betas: &[Json], name: &str) -> bool {
    betas
        .iter()
        .any(|value| matches!(value, Json::Text(text) if text.as_str() == name))
}

fn beta_value(value: BetaValue) -> Json {
    match value {
        BetaValue::Null => Json::Null,
        BetaValue::Bool(flag) => Json::Bool(flag),
        BetaValue::Number(number) => Json::Number(number),
        BetaValue::Text(text) => Json::Text(text),
        BetaValue::List(items) => Json::List(items.into_iter().map(beta_value).collect()),
        BetaValue::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, entry)| (name, beta_value(entry)))
                .collect(),
        ),
    }
}
