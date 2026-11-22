use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_db::resource_repository::table::KeyFilter;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};

use super::error::ModelFieldError;

// MirroredPricingParams in litellm/types/utils.py: the prices Deployment.__init__ copies onto model_info
const MIRRORED_PRICING_FIELDS: [&str; 7] = [
    "input_cost_per_token",
    "output_cost_per_token",
    "input_cost_per_character",
    "output_cost_per_character",
    "cache_read_input_token_cost",
    "cache_creation_input_token_cost",
    "tiered_pricing",
];

// A LiteLLM_ProxyModelTable row as the Deployment the update paths judge: params stay as stored, encrypted values included.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StoredDeployment {
    pub model_id: String,
    pub model_name: String,
    pub litellm_params: JsonObject,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(default)]
    pub model_info: JsonObject,
    #[serde(default)]
    pub blocked: bool,
}

pub async fn get_db_model(
    model_id: &str,
    models: &ModelRepository<'_>,
) -> Result<Option<StoredDeployment>, ModelFieldError> {
    let rows: Vec<ProxyModelTable> = models
        .table()?
        .as_ref()
        .find(KeyFilter::Equal(model_id))
        .await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let encoded =
        serde_json::to_string(&row.fields).map_err(|source| ModelFieldError::Row { source })?;
    let mut deployment: StoredDeployment =
        serde_json::from_str(&encoded).map_err(|source| ModelFieldError::Row { source })?;
    for field in MIRRORED_PRICING_FIELDS {
        if let Some(price) = deployment
            .litellm_params
            .get(field)
            .filter(|price| !matches!(price, Json::Null))
        {
            deployment
                .model_info
                .insert(field.to_owned(), price.clone());
        }
    }
    Ok(Some(deployment))
}
