use deadpool_postgres::GenericClient;
use tokio_postgres::Row;
use tokio_postgres::types::Json;

use super::StoreError;
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;
use crate::ptu::charges::deployment::{
    LoadedDeployments, RouterEntry, StoredModel, scan_deployments,
};

const SCAN_SQL: &str =
    "SELECT \"model_id\", \"model_name\", \"model_info\" FROM \"LiteLLM_ProxyModelTable\"";

pub async fn load_models<C>(
    client: &C,
    entries: &[RouterEntry],
) -> Result<LoadedDeployments, StoreError>
where
    C: GenericClient + Sync,
{
    let rows = client
        .query(SCAN_SQL, &[])
        .await
        .map_err(|error| StoreError::Scan(backend(error)))?;
    let stored = rows
        .iter()
        .map(stored_model)
        .collect::<Result<Vec<StoredModel>, StoreError>>()?;
    Ok(scan_deployments(&stored, entries))
}

fn stored_model(row: &Row) -> Result<StoredModel, StoreError> {
    let model_id: String = row
        .try_get("model_id")
        .map_err(|error| StoreError::Scan(backend(error)))?;
    let model_name: String = row
        .try_get("model_name")
        .map_err(|error| StoreError::Scan(backend(error)))?;
    let model_info: Option<Json<JsonData>> = row
        .try_get("model_info")
        .map_err(|error| StoreError::Scan(backend(error)))?;
    Ok(StoredModel {
        model_id,
        model_name,
        model_info: model_info.map(|wrapped| wrapped.0),
    })
}
