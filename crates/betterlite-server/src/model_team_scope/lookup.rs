use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::model_registration::JsonObject;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use tokio::task::JoinError;
use tokio_util::task::TaskTracker;

use crate::db_row_decrypt::deployments::decrypt_model_rows;
use crate::db_row_decrypt::error::DbRowDecryptError;
use crate::db_row_decrypt::warnings::Warnings;
use crate::health_access_and_connections::visibility::HealthRouter;
use crate::model_info_scope::text_field;

use super::error::ScopeError;
use super::reads::TeamScopeReads;

// A secret read opens token files and waits on OIDC providers, so the inputs are shared and outlive the blocking task that decrypts.
#[derive(Clone)]
pub struct Decryption {
    pub cipher: Arc<DbCipher>,
    pub secrets: Arc<SecretLookup>,
    pub environment: Arc<BTreeMap<String, String>>,
    pub warnings: Arc<Warnings>,
    // Inject the server's shared tracker so shutdown drains secret reads after request cancellation.
    pub tasks: TaskTracker,
}

#[derive(Debug, thiserror::Error)]
enum DecryptTaskError {
    #[error(transparent)]
    Decrypt(#[from] DbRowDecryptError),
    #[error("the decryption task failed: {0}")]
    Task(#[from] JoinError),
}

pub async fn find_model_by_id<Reads>(
    model_id: &str,
    search: Option<&str>,
    router: Option<&dyn HealthRouter>,
    reads: &Reads,
    decrypt: &Decryption,
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

async fn stored_model<Reads>(
    model_id: &str,
    reads: &Reads,
    decrypt: &Decryption,
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
    match decrypted_row(row, Decryption::clone(decrypt)).await {
        Ok(deployments) => deployments.into_iter().next(),
        Err(error) => {
            tracing::error!("Error querying database for modelId {model_id}: {error}");
            None
        }
    }
}

// The blocking pool, so a token file or an OIDC provider never holds an async worker.
async fn decrypted_row(
    row: ProxyModelTable,
    request: Decryption,
) -> Result<Vec<JsonObject>, DecryptTaskError> {
    let rows = [row];
    let tasks = request.tasks.clone();
    let task = tasks.spawn(async move {
        tokio::task::spawn_blocking(move || {
            decrypt_model_rows(
                &rows,
                &request.cipher,
                &request.secrets,
                &request.environment,
                &request.warnings,
            )
        })
        .await
    });
    let decrypt_result = task.await??;
    decrypt_result.map_err(DecryptTaskError::from)
}

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
