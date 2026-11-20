use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::router_dependencies::{StrategyRouterKind, classify_strategy_router_model};

// decrypt_value_helper(exception_type="debug", return_original_value=True): a failed decrypt keeps the stored text.
const KEEP_ORIGINAL: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Original,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AutoRouterCatalogEntry {
    pub model_id: String,
    pub team_id: Option<String>,
    pub created_by: Option<String>,
    pub deployment: CatalogDeployment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogDeployment {
    pub litellm_params: CatalogLitellmParams,
    pub model_info: CatalogModelInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogLitellmParams {
    pub model: String,
    pub complexity_router_config: Option<JsonData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogModelInfo {
    pub id: String,
    pub db_model: bool,
}

pub trait AutoRouterCatalog: Send + Sync {
    fn catalog(&self) -> Option<Arc<Vec<AutoRouterCatalogEntry>>>;
}

#[must_use]
pub fn build_auto_router_catalog(
    rows: &[ProxyModelTable],
    cipher: &DbCipher,
) -> Option<Vec<AutoRouterCatalogEntry>> {
    let mut catalog = Vec::new();
    for row in rows {
        let team_id = match row
            .fields
            .model_info
            .as_ref()
            .and_then(|info| info.0.get("team_id"))
        {
            None | Some(JsonData::Null) => None,
            Some(JsonData::String(team)) => Some(team.clone()),
            Some(
                JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
            ) => return None,
        };
        if let Some(entry) = catalog_entry(row, team_id, cipher) {
            catalog.push(entry);
        }
    }
    Some(catalog)
}

fn catalog_entry(
    row: &ProxyModelTable,
    team_id: Option<String>,
    cipher: &DbCipher,
) -> Option<AutoRouterCatalogEntry> {
    let model_field = catalog_field(row.fields.litellm_params.0.get("model"), "model", cipher);
    let JsonData::String(model) = model_field else {
        return None;
    };
    if classify_strategy_router_model(&model) != Some(StrategyRouterKind::Complexity) {
        return None;
    }
    let config_field = catalog_field(
        row.fields.litellm_params.0.get("complexity_router_config"),
        "complexity_router_config",
        cipher,
    );
    // the deployment always carries the key, so a config the row does not store reads null
    let complexity_router_config = if matches!(config_field, JsonData::Null) {
        None
    } else {
        Some(config_field)
    };
    Some(AutoRouterCatalogEntry {
        model_id: row.fields.model_id.clone(),
        team_id,
        created_by: row.fields.created_by.clone(),
        deployment: CatalogDeployment {
            litellm_params: CatalogLitellmParams {
                model,
                complexity_router_config,
            },
            model_info: CatalogModelInfo {
                id: row.fields.model_id.clone(),
                db_model: true,
            },
        },
    })
}

fn catalog_field(value: Option<&JsonData>, field: &str, cipher: &DbCipher) -> JsonData {
    let Some(value) = value else {
        return JsonData::Null;
    };
    let JsonData::String(text) = value else {
        return value.clone();
    };
    let stored = StoredValue::Text(SecretString::from(text.clone()));
    match cipher.decrypt_value(stored, field, KEEP_ORIGINAL) {
        Some(StoredValue::Text(plain)) => JsonData::String(plain.expose_secret().to_owned()),
        Some(StoredValue::Other(other)) => other,
        None => JsonData::Null,
    }
}

mod tests;
