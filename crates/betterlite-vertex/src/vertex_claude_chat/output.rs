use std::collections::BTreeSet;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::capabilities::effort_param::model_supports_effort_param;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

use super::PROVIDER;

pub const OUTPUT_CONFIG: &str = "output_config";
pub const EFFORT: &str = "effort";

// Keys Vertex AI Claude rejects whatever the model; the set is empty at the pin.
pub const UNSUPPORTED_OUTPUT_CONFIG_KEYS: [&str; 0] = [];

// output_format passes untouched; output_config keeps every key but an unsupported effort and vanishes when empty.
pub async fn sanitize_output_params<Source, Transport>(
    body: &mut IndexMap<String, Json>,
    model: &str,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let fields = match body.get(OUTPUT_CONFIG).cloned() {
        None | Some(Json::Null) => return,
        Some(Json::Object(fields)) => fields,
        Some(_) => {
            body.shift_remove(OUTPUT_CONFIG);
            return;
        }
    };
    let mut drop_keys: BTreeSet<&str> = UNSUPPORTED_OUTPUT_CONFIG_KEYS.into_iter().collect();
    if fields.contains_key(EFFORT)
        && !model_supports_effort_param(model, PROVIDER, environment, catalogs).await
    {
        drop_keys.insert(EFFORT);
    }
    let sanitized: IndexMap<String, Json> = fields
        .into_iter()
        .filter(|(key, _value)| !drop_keys.contains(key.as_str()))
        .collect();
    if sanitized.is_empty() {
        body.shift_remove(OUTPUT_CONFIG);
    } else {
        body.insert(OUTPUT_CONFIG.to_owned(), Json::Object(sanitized));
    }
}
