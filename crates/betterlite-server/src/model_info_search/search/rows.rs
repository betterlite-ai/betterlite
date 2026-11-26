use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tokio::task::{JoinError, JoinSet};

use crate::db_row_decrypt::deployments::decrypt_model_rows;
use crate::db_row_decrypt::error::DbRowDecryptError;
use crate::model_team_scope::caller_scope::byok_row_outside_caller_teams;
use crate::model_team_scope::lookup::Decryption;

#[derive(Debug, thiserror::Error)]
pub(super) enum DecryptTaskError {
    #[error(transparent)]
    Decrypt(#[from] DbRowDecryptError),
    #[error("the decryption task failed: {0}")]
    Task(#[from] JoinError),
    #[error("the decryption task never ran")]
    Missing,
}

// The raw row is scoped before decrypt_model_list_from_db touches any credential in it.
pub(super) fn row_outside_caller_teams(
    row: &ProxyModelTable,
    allowed: Option<&BTreeSet<String>>,
) -> bool {
    let Some(payload) = &row.fields.model_info else {
        return false;
    };
    byok_row_outside_caller_teams(&stored_model_info(payload), allowed)
}

fn stored_model_info(payload: &Payload) -> JsonObject {
    payload
        .0
        .iter()
        .map(|(key, value)| (key.clone(), stored_json(value)))
        .collect()
}

fn stored_json(value: &JsonData) -> Json {
    match value {
        JsonData::Null => Json::Null,
        JsonData::Bool(flag) => Json::Bool(*flag),
        JsonData::Number(number) => Json::Number(number.clone()),
        JsonData::String(text) => Json::Text(text.clone()),
        JsonData::Array(items) => Json::List(items.iter().map(stored_json).collect()),
        JsonData::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), stored_json(item)))
                .collect(),
        ),
    }
}

pub(super) async fn decrypt_batch(
    rows: Vec<ProxyModelTable>,
    request: Decryption,
) -> Result<Vec<JsonObject>, DecryptTaskError> {
    let tasks = request.tasks.clone();
    let task = tasks.spawn(async move {
        let mut blocking = JoinSet::new();
        blocking.spawn_blocking(move || {
            decrypt_model_rows(
                &rows,
                &request.cipher,
                &request.secrets,
                &request.environment,
                &request.warnings,
            )
        });
        match blocking.join_next().await {
            Some(Ok(decrypted)) => decrypted.map_err(DecryptTaskError::from),
            Some(Err(error)) => Err(DecryptTaskError::from(error)),
            None => Err(DecryptTaskError::Missing),
        }
    });
    task.await.map_err(DecryptTaskError::from)?
}
