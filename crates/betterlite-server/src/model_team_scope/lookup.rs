use std::collections::BTreeMap;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::model_registration::JsonObject;
use betterlite_secret_managers::secret_lookup::SecretLookup;

use crate::db_row_decrypt::deployments::decrypt_model_rows;
use crate::db_row_decrypt::warnings::Warnings;
use crate::health_access_and_connections::visibility::HealthRouter;
use crate::model_info_scope::text_field;

use super::error::ScopeError;
use super::reads::TeamScopeReads;

// ProxyConfig.decrypt_model_list_from_db's inputs: the cipher, the secret manager, the request's environment and the once-only warnings.
pub struct Decryption<'a> {
    pub cipher: &'a DbCipher,
    pub secrets: &'a SecretLookup,
    pub environment: &'a BTreeMap<String, String>,
    pub warnings: &'a Warnings,
}

// _find_model_by_id: the configured deployment, else the stored row decrypted; a search term then keeps only a name that contains it.
pub async fn find_model_by_id<Reads>(
    model_id: &str,
    search: Option<&str>,
    router: Option<&dyn HealthRouter>,
    reads: &Reads,
    decrypt: &Decryption<'_>,
) -> Result<(Vec<JsonObject>, usize), ScopeError>
where
    Reads: TeamScopeReads + ?Sized,
{
    let found = match configured_model(model_id, router)? {
        Some(model) => Some(model),
        None => stored_model(model_id, reads, decrypt).await,
    };
    let models: Vec<JsonObject> = found
        .and_then(|model| search_keeps(model, search))
        .into_iter()
        .collect();
    let count = models.len();
    Ok((models, count))
}

// get_model_info(id=model_id): the router's copy of the deployment, absent when the id names no configured model.
fn configured_model(
    model_id: &str,
    router: Option<&dyn HealthRouter>,
) -> Result<Option<JsonObject>, ScopeError> {
    let Some(health) = router else {
        return Ok(None);
    };
    Ok(health
        .get_deployment(model_id)?
        .map(|deployment| deployment.to_json()))
}

// The database fallback: ProxyConfig.decrypt_model_list_from_db's first decrypted row, none when the read or the decryption fails.
async fn stored_model<Reads>(
    model_id: &str,
    reads: &Reads,
    decrypt: &Decryption<'_>,
) -> Option<JsonObject>
where
    Reads: TeamScopeReads + ?Sized,
{
    let row = match reads.model_row(model_id).await {
        Ok(row) => row?,
        Err(error) => {
            tracing::error!("Error querying database for modelId {model_id}: {error}");
            return None;
        }
    };
    let rows = [row];
    match decrypt_model_rows(
        &rows,
        decrypt.cipher,
        decrypt.secrets,
        decrypt.environment,
        decrypt.warnings,
    ) {
        Ok(deployments) => deployments.into_iter().next(),
        Err(error) => {
            tracing::error!("Error querying database for modelId {model_id}: {error}");
            None
        }
    }
}

// The search arm: a blank term keeps the model, and any other term must appear in the model's lowercased name.
fn search_keeps(model: JsonObject, search: Option<&str>) -> Option<JsonObject> {
    let Some(term) = search else {
        return Some(model);
    };
    let lowered = term.to_lowercase();
    let needle = lowered.trim();
    if needle.is_empty() {
        return Some(model);
    }
    let name = text_field(&model, "model_name").unwrap_or_default();
    let keeps = name.to_lowercase().contains(needle);
    keeps.then_some(model)
}
