use async_trait::async_trait;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::user_repository::{RowDecodeError, UserRepository};
use rust_decimal::Decimal;

use super::global_spend::ProxyBudgetRows;

#[derive(Debug, thiserror::Error)]
pub enum SpendReadError {
    #[error(transparent)]
    Deadline(#[from] DbLookupError),
    #[error(transparent)]
    Session(#[from] DbError),
    #[error(transparent)]
    Row(#[from] DbError<RowDecodeError>),
}

#[async_trait]
impl ProxyBudgetRows for PostgresStore {
    async fn find_proxy_budget(&self) -> Result<Option<Decimal>, SpendReadError> {
        let session = self.session().await?;
        let repository = UserRepository {
            db: Some(&session),
            writer_db: None,
            use_writer: false,
        };
        let row = repository.find_by_id("litellm-proxy-budget").await?;
        Ok(row.map(|user| user.spend))
    }
}
