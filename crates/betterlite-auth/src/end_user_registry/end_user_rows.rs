use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;

use super::predicate::restricted_end_user_sql;

// Prisma's find_many(where=_restricted_end_user_where(), take=bound+1) over B726's end-user table
pub(super) async fn restricted_end_user_ids(
    store: &PostgresStore,
    take: i64,
) -> Result<Vec<String>, DbError> {
    let statement = restricted_end_user_sql();
    let client = store.session().await?;
    let rows = client
        .query(statement.as_str(), &[&take])
        .await
        .map_err(backend)?;
    rows.iter()
        .map(|row| row.try_get(0).map_err(backend))
        .collect()
}
