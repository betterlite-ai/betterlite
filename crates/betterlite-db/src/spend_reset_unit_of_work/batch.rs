use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use tokio_postgres::types::ToSql;

use super::queue::{BudgetIdFilter, CascadeEntry, LinkedResetFilter, LinkedWrite, SpendResetEntry};
use super::{BudgetCascadeUnitOfWork, SpendResetUnitOfWork};
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error("queued statement failed")]
    Statement(#[source] DbError),
    #[error("no row matched the reset target in {table}")]
    MissingResetTarget { table: &'static str },
    #[error("commit rejected by the database")]
    Rejected(#[source] DbError),
    #[error("commit outcome is unknown")]
    OutcomeUnknown(#[source] DbError),
}

fn classify_commit_failure(source: tokio_postgres::Error) -> CommitError {
    let db_error = backend(source);
    match db_error {
        DbError::UniqueViolation { .. } | DbError::SqlState { .. } => {
            CommitError::Rejected(db_error)
        }
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::Backend { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull => CommitError::OutcomeUnknown(db_error),
    }
}

pub struct SpendResetBatch {
    pub store: PostgresStore,
}

pub struct BudgetCascadeBatch {
    pub store: PostgresStore,
}

impl SpendResetBatch {
    pub async fn commit(&self, unit: SpendResetUnitOfWork) -> Result<(), CommitError> {
        let entries = unit.keys.queue.take();
        let mut client = self.store.session().await.map_err(CommitError::Statement)?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| CommitError::Statement(backend(source)))?;
        for entry in &entries {
            apply_single_reset(&transaction, entry).await?;
        }
        transaction.commit().await.map_err(classify_commit_failure)
    }
}

impl BudgetCascadeBatch {
    pub async fn commit(&self, unit: BudgetCascadeUnitOfWork) -> Result<(), CommitError> {
        let entries = unit.budgets.queue.take();
        let mut client = self.store.session().await.map_err(CommitError::Statement)?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| CommitError::Statement(backend(source)))?;
        for entry in &entries {
            match entry {
                CascadeEntry::Linked { table, write } => {
                    apply_linked_write(&transaction, table, write).await?;
                }
                CascadeEntry::Window {
                    budget_id,
                    budget_reset_at,
                } => {
                    apply_window_advance(&transaction, budget_id, *budget_reset_at).await?;
                }
            }
        }
        transaction.commit().await.map_err(classify_commit_failure)
    }
}

async fn apply_single_reset<C>(client: &C, entry: &SpendResetEntry) -> Result<(), CommitError>
where
    C: GenericClient + Sync,
{
    let reset_at = entry.data.budget_reset_at.map(|value| value.naive_utc());
    let sql = format!(
        "UPDATE \"{table}\" SET spend = spend - $1::numeric, budget_reset_at = $2 WHERE {column} = $3 RETURNING {column}",
        table = entry.table,
        column = entry.column,
    );
    client
        .query_opt(
            sql.as_str(),
            &[&entry.data.spend.decrement, &reset_at, &entry.identifier],
        )
        .await
        .map_err(|source| CommitError::Statement(backend(source)))?
        .ok_or(CommitError::MissingResetTarget { table: entry.table })?;
    Ok(())
}

async fn apply_window_advance<C>(
    client: &C,
    budget_id: &str,
    budget_reset_at: DateTime<Utc>,
) -> Result<(), CommitError>
where
    C: GenericClient + Sync,
{
    let reset_at = budget_reset_at.naive_utc();
    client
        .execute(
            "UPDATE \"LiteLLM_BudgetTable\" SET budget_reset_at = $1 WHERE budget_id = $2",
            &[&reset_at, &budget_id],
        )
        .await
        .map_err(|source| CommitError::Statement(backend(source)))?;
    Ok(())
}

fn render_filter(filter: &LinkedResetFilter) -> (String, Vec<Box<dyn ToSql + Sync + Send>>) {
    let mut clauses = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Sync + Send>> = Vec::new();
    match &filter.budget_id {
        BudgetIdFilter::Equals(value) => {
            params.push(Box::new(value.clone()));
            clauses.push(format!("budget_id = ${}", params.len()));
        }
        BudgetIdFilter::IsNull => clauses.push("budget_id IS NULL".to_owned()),
    }
    if let Some(range) = &filter.spend {
        if let Some(bound) = range.greater_than {
            params.push(Box::new(bound));
            clauses.push(format!("spend > ${}::numeric", params.len()));
        }
        if let Some(bound) = range.less_than_or_equal {
            params.push(Box::new(bound));
            clauses.push(format!("spend <= ${}::numeric", params.len()));
        }
    }
    (clauses.join(" AND "), params)
}

async fn apply_linked_write<C>(
    client: &C,
    table: &'static str,
    write: &LinkedWrite,
) -> Result<(), CommitError>
where
    C: GenericClient + Sync,
{
    match write {
        LinkedWrite::Zero(filter) => {
            let (where_sql, params) = render_filter(filter);
            let sql = format!("UPDATE \"{table}\" SET spend = 0 WHERE {where_sql}");
            let refs: Vec<&(dyn ToSql + Sync)> = params
                .iter()
                .map(|value| -> &(dyn ToSql + Sync) { value.as_ref() })
                .collect();
            client
                .query_opt(sql.as_str(), &refs)
                .await
                .map_err(|source| CommitError::Statement(backend(source)))?;
        }
        LinkedWrite::Decrement(filter, amount) => {
            let (where_sql, mut params) = render_filter(filter);
            let amount_index = params.len().saturating_add(1);
            params.push(Box::new(*amount));
            let sql = format!(
                "UPDATE \"{table}\" SET spend = spend - ${amount_index}::numeric WHERE {where_sql}"
            );
            let refs: Vec<&(dyn ToSql + Sync)> = params
                .iter()
                .map(|value| -> &(dyn ToSql + Sync) { value.as_ref() })
                .collect();
            client
                .query_opt(sql.as_str(), &refs)
                .await
                .map_err(|source| CommitError::Statement(backend(source)))?;
        }
    }
    Ok(())
}
