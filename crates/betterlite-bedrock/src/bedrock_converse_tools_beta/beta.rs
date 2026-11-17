use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::feature_flags::beta::EFFORT_BETA;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

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

#[must_use]
pub fn has_beta(betas: &[Json], name: &str) -> bool {
    betas
        .iter()
        .any(|value| matches!(value, Json::Text(text) if text.as_str() == name))
}

pub async fn push_effort_beta<Source, Transport>(
    params: &IndexMap<String, Json>,
    model: &str,
    betas: &mut Vec<Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let Some(Json::Object(output_config)) = params.get(OUTPUT_CONFIG) else {
        return;
    };
    if output_config
        .get(EFFORT)
        .is_none_or(|effort| matches!(effort, Json::Null))
    {
        return;
    }
    if supports_model_capability(
        model,
        ADAPTIVE_THINKING,
        BEDROCK_PROVIDER,
        environment,
        catalogs,
    )
    .await
    {
        return;
    }
    if !has_beta(betas, EFFORT_BETA) {
        betas.push(Json::Text(EFFORT_BETA.to_owned()));
    }
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
