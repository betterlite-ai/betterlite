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
    if !alias_exists(store, alias, excluded).await? {
        return Ok(());
    }
    let message = format!(
        "Key with alias '{alias}' already exists. Unique key aliases across all keys are required."
    );
    Err(AliasUniquenessError::Duplicate(bad_request_for_alias(
        message,
    )))
}

async fn alias_exists(
    store: &PostgresStore,
    alias: &str,
    excluded_token: Option<&str>,
) -> Result<bool, DbError> {
    let client = store.session().await?;
    let sql = if excluded_token.is_some() {
        format!(r#"SELECT 1 FROM "{KEY_TABLE}" WHERE key_alias = $1 AND NOT token = $2 LIMIT 1"#)
    } else {
        format!(r#"SELECT 1 FROM "{KEY_TABLE}" WHERE key_alias = $1 LIMIT 1"#)
    };
    let parameters: Vec<&(dyn ToSql + Sync)> = match excluded_token {
        Some(token) => vec![&alias, &token],
        None => vec![&alias],
    };
    let row = client
        .query_opt(sql.as_str(), &parameters)
        .await
        .map_err(backend)?;
    Ok(row.is_some())
}
