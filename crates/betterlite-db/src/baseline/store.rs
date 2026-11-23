use std::future::Future;
use std::time::Duration;

use betterlite_models::savings_pricing::context::SavingsPricingContext;
use deadpool_postgres::{Client, Transaction};
use uuid::Uuid;

use crate::baseline::flush::claim::claim_dirty;
use crate::baseline::publication::pricing::PublicationPricing;
use crate::baseline::records::BaselineAccountingRecord;
use crate::database_replica::pinned::WriterPinnedClient;
use crate::postgres_store::session::PostgresStore;

use self::error::StoreError;

pub mod append;
pub mod error;
pub mod pages;
pub mod project;
pub mod retire;
pub mod sql;

// Prisma's interactive transaction deadline on every accounting transaction the source opens.
pub const TRANSACTION_TIMEOUT: Duration = Duration::from_secs(10);
pub const ACCOUNTING_LIMITS: &str =
    "SET LOCAL statement_timeout = 5000; SET LOCAL lock_timeout = 1000";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendResult {
    Recorded,
    Retired,
    Conflict,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectResult {
    Published,
    Unchanged,
    Unavailable,
}

// Transactions open on the writer B714 pins, since a replica may lag the comparison row they lock.
pub struct BaselineAccountingStore<'db, C, G> {
    pub database: WriterPinnedClient<'db, PostgresStore>,
    pub pricing: PublicationPricing<'db, C>,
    pub generate_id: G,
}

// The three durable operations a flush drives, so its leasing and claiming can run against a fake.
pub trait BaselineAccounting: Sync {
    fn append(
        &self,
        record: &BaselineAccountingRecord,
    ) -> impl Future<Output = AppendResult> + Send;

    fn project(&self, scope: &str) -> impl Future<Output = ProjectResult> + Send;

    fn claim_dirty(&self) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;
}

impl<C, G> BaselineAccounting for BaselineAccountingStore<'_, C, G>
where
    C: SavingsPricingContext + Sync,
    G: Fn() -> Uuid + Sync,
{
    fn append(
        &self,
        record: &BaselineAccountingRecord,
    ) -> impl Future<Output = AppendResult> + Send {
        append::append_record(self, record)
    }

    fn project(&self, scope: &str) -> impl Future<Output = ProjectResult> + Send {
        project::project_scope(self, scope)
    }

    fn claim_dirty(&self) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send {
        claim_dirty(self.database)
    }
}

// SET LOCAL runs before any statement, so the bounds hold for every lock the transaction takes.
pub async fn begin<'session>(
    session: &'session mut Client,
    limits: &str,
) -> Result<Transaction<'session>, tokio_postgres::Error> {
    let transaction = session.transaction().await?;
    transaction.batch_execute(limits).await?;
    Ok(transaction)
}

pub(crate) mod tests;
