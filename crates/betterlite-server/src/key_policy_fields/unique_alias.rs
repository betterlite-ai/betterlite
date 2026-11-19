use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::table_ops::access::KEY_TABLE;
use betterlite_error::proxy_exception::ProxyException;
use tokio_postgres::types::ToSql;

use crate::key_policy_fields::bad_request_for_alias;

#[derive(Debug, thiserror::Error)]
pub enum AliasUniquenessError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("{0}")]
    Duplicate(Box<ProxyException>),
}

// a global find_first on the alias; a truthy key token excludes that one row and is never hashed here
pub async fn enforce_unique_key_alias(
    key_alias: Option<&str>,
    store: Option<&PostgresStore>,
    existing_key_token: Option<&str>,
) -> Result<(), AliasUniquenessError> {
    let Some(alias) = key_alias else {
        return Ok(());
    };
    let Some(store) = store else {
        return Ok(());
    };
    let excluded = existing_key_token.filter(|token| !token.is_empty());
    let sql = format!(
        r#"SELECT 1 FROM "{KEY_TABLE}" WHERE key_alias = $1 AND ($2::text IS NULL OR NOT token = $2) LIMIT 1"#
    );
    let parameters: Vec<&(dyn ToSql + Sync)> = vec![&alias, &excluded];
    let row = store
        .session()
        .await?
        .query_opt(sql.as_str(), &parameters)
        .await
        .map_err(backend)?;
    if row.is_none() {
        return Ok(());
    }
    Err(AliasUniquenessError::Duplicate(bad_request_for_alias(
        format!(
            "Key with alias '{alias}' already exists. Unique key aliases across all keys are required."
        ),
    )))
}
