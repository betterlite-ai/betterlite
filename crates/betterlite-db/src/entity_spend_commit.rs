pub mod balances;
pub mod membership;
pub mod progress;

mod tests;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use rust_decimal::Decimal;
use thiserror::Error;

use self::membership::MemberKeyError;
use crate::postgres_store::errors::DbError;
use crate::spend_commit::policy::{SpendFailure, SpendUpdateFailure, classify_spend_failure};
use crate::spend_commit::tables::SpendTable;

pub type TableFuture<'writer, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'writer>>;

// _spend_update_tx for one entity table: every increment of the table applies in one transaction, or none of them does.
pub trait EntityTableWriter: Send + Sync {
    type Error: SpendUpdateFailure;

    fn write_table<'writer>(
        &'writer self,
        table: SpendTable,
        increments: &'writer BTreeMap<String, Decimal>,
    ) -> TableFuture<'writer, Self::Error>;
}

#[derive(Debug, Error)]
pub enum EntityTableError {
    #[error("the {} spend update failed", .table.name())]
    Statement {
        table: SpendTable,
        #[source]
        source: DbError,
    },
    #[error("the {} spend update holds a malformed member key", .table.name())]
    Key {
        table: SpendTable,
        #[source]
        source: MemberKeyError,
    },
    #[error("the {} spend transaction timed out after {seconds}s before its commit", .table.name())]
    Timeout { table: SpendTable, seconds: u64 },
    #[error("the {} spend commit timed out after {seconds}s and may have applied", .table.name())]
    CommitTimeout { table: SpendTable, seconds: u64 },
}

impl SpendUpdateFailure for EntityTableError {
    fn spend_failure(&self) -> SpendFailure {
        match self {
            Self::Statement { source, .. } => classify_spend_failure(source),
            Self::Key { .. } | Self::Timeout { .. } => SpendFailure::Other,
            // the commit was sent when the deadline fired, so the increments may already be applied
            Self::CommitTimeout { .. } => SpendFailure::Ambiguous,
        }
    }
}
