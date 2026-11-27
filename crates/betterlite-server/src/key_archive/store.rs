mod parameters;
mod sql;

use betterlite_db::identity_rows::key::DeletedVerificationToken;
use betterlite_db::postgres_store::errors::backend;
use deadpool_postgres::GenericClient;
use tokio_postgres::types::ToSql;

use self::parameters::{Parameter, archived_parameters};
use super::KeyArchiveError;

mod tests;

pub async fn save_deleted_verification_token_records<C: GenericClient + Sync>(
    db: &C,
    records: &[DeletedVerificationToken],
) -> Result<(), KeyArchiveError> {
    if records.is_empty() {
        return Ok(());
    }
    let statement = sql::insert_statement(records.len());
    let parameters = archived_parameters(records)?;
    let bound: Vec<&(dyn ToSql + Sync)> = parameters.iter().map(Parameter::bind).collect();
    db.execute(statement.as_str(), bound.as_slice())
        .await
        .map_err(|driver| KeyArchiveError::Database {
            source: backend(driver),
        })?;
    Ok(())
}
