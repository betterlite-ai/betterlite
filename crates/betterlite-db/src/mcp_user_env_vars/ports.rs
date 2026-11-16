use std::future::Future;
use std::pin::Pin;

use crate::budget_permission_rows::JsonData;

use super::error::UserEnvVarsError;
use super::store::UserEnvVars;

// every transaction call yields one boxed future, so the trait stays object safe like LiteLLM's Protocol
pub type TransactionFuture<'tx, T> =
    Pin<Box<dyn Future<Output = Result<T, UserEnvVarsError>> + Send + 'tx>>;

pub trait UserEnvVarTable: Send + Sync {
    fn find_unique(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, UserEnvVarsError>> + Send;

    fn find_many(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> impl Future<Output = Result<Vec<JsonData>, UserEnvVarsError>> + Send;

    fn delete_many(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), UserEnvVarsError>> + Send;
}

pub trait UserEnvVarsTransaction: Send + Sync {
    fn execute_raw<'tx>(&'tx self, statement: &'tx str, key: i64) -> TransactionFuture<'tx, ()>;

    fn find_unique<'tx>(
        &'tx self,
        user_id: &'tx str,
        server_id: &'tx str,
    ) -> TransactionFuture<'tx, Option<JsonData>>;

    fn upsert<'tx>(
        &'tx self,
        user_id: &'tx str,
        server_id: &'tx str,
        values_b64: &'tx str,
    ) -> TransactionFuture<'tx, ()>;
}

// the read-modify-write merge runs on the locked transaction, as LiteLLM's `async with` block does
pub trait UserEnvVarsMergeStep: Send + Sync {
    fn apply<'tx>(
        &'tx self,
        transaction: &'tx dyn UserEnvVarsTransaction,
    ) -> TransactionFuture<'tx, UserEnvVars>;
}

pub trait UserEnvVarsTransactions: Send + Sync {
    fn in_transaction<'tx>(
        &'tx self,
        step: &'tx dyn UserEnvVarsMergeStep,
    ) -> TransactionFuture<'tx, UserEnvVars>;
}
