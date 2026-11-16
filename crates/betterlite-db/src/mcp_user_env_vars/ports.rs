use std::future::Future;
use std::pin::Pin;

use crate::budget_permission_rows::JsonData;

use super::error::UserEnvVarsError;
use super::store::UserEnvVars;

// every transaction call yields one boxed future, so the trait stays object safe like LiteLLM's Protocol
pub type TransactionFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, UserEnvVarsError>> + Send + 'a>>;

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
    ) -> impl Future<Output = Result<usize, UserEnvVarsError>> + Send;
}

pub trait UserEnvVarsTransaction: Send + Sync {
    fn execute_raw<'a>(&'a self, statement: &'a str, key: i64) -> TransactionFuture<'a, ()>;

    fn find_unique<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
    ) -> TransactionFuture<'a, Option<JsonData>>;

    fn upsert<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
        values_b64: &'a str,
    ) -> TransactionFuture<'a, ()>;
}

// the read-modify-write merge runs on the locked transaction, as LiteLLM's `async with` block does
pub trait UserEnvVarsMergeStep: Send + Sync {
    fn apply<'tx>(
        &'tx self,
        transaction: &'tx dyn UserEnvVarsTransaction,
    ) -> TransactionFuture<'tx, UserEnvVars>;
}

pub trait UserEnvVarsTransactions: Send + Sync {
    fn in_transaction<'a>(
        &'a self,
        step: &'a dyn UserEnvVarsMergeStep,
    ) -> TransactionFuture<'a, UserEnvVars>;
}
