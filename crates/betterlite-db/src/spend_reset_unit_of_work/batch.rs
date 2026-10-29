use chrono::NaiveDateTime;
use deadpool_postgres::GenericClient;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tokio_postgres::types::ToSql;

use super::queue::{BudgetIdFilter, LinkedResetFilter, LinkedWrite, SingleReset, WindowAdvance};
use super::{BudgetCascadeUnitOfWork, SpendResetUnitOfWork};
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error("queued statement failed")]
    Statement(#[source] DbError),
    #[error("no row matched the reset target in {table}")]
    MissingResetTarget { table: &'static str },
    #[error("decimal {value} has no exact floating point representation")]
    Money { value: Decimal },
    #[error("commit outcome is unknown")]
    Commit(#[source] DbError),
}

fn to_f64(value: Decimal) -> Result<f64, CommitError> {
    value.to_f64().ok_or(CommitError::Money { value })
}

async fn run_statement<C>(
    client: &C,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Option<tokio_postgres::Row>, CommitError>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(sql, params)
        .await
        .map_err(|source| CommitError::Statement(backend(source)))
}

fn bind_params(params: &[Box<dyn ToSql + Sync + Send>]) -> Vec<&(dyn ToSql + Sync)> {
    params
        .iter()
        .map(|value| -> &(dyn ToSql + Sync) { value.as_ref() })
        .collect()
}

pub struct SpendResetBatch {
    pub store: PostgresStore,
}

pub struct BudgetCascadeBatch {
    pub store: PostgresStore,
}

macro_rules! write_group_commit {
    ($batch:ty, $unit_param:ident : $unit_ty:ty, [$($group:expr),+ $(,)?]) => {
        impl $batch {
            pub async fn commit(&self, $unit_param: $unit_ty) -> Result<(), CommitError> {
                let groups = [$($group),+];
                commit_groups(&self.store, &groups).await
            }
        }
    };
}

write_group_commit!(
    SpendResetBatch,
    unit: SpendResetUnitOfWork,
    [
        WriteGroup::Single("LiteLLM_VerificationToken", "token", &unit.keys.queued),
        WriteGroup::Single("LiteLLM_UserTable", "user_id", &unit.users.queued),
        WriteGroup::Single("LiteLLM_TeamTable", "team_id", &unit.teams.queued),
    ]
);

write_group_commit!(
    BudgetCascadeBatch,
    unit: BudgetCascadeUnitOfWork,
    [
        WriteGroup::Linked("LiteLLM_TeamMembership", &unit.team_memberships.queued),
        WriteGroup::Linked("LiteLLM_VerificationToken", &unit.keys.queued),
        WriteGroup::Linked("LiteLLM_OrganizationTable", &unit.organizations.queued),
        WriteGroup::Linked("LiteLLM_TagTable", &unit.tags.queued),
        WriteGroup::Linked(
            "LiteLLM_ModelAccessGroupBudgetTable",
            &unit.model_access_groups.queued,
        ),
        WriteGroup::Linked("LiteLLM_ProjectTable", &unit.projects.queued),
        WriteGroup::Linked("LiteLLM_EndUserTable", &unit.endusers.queued),
        WriteGroup::Window(&unit.budgets.queued),
    ]
);

enum WriteGroup<'queue> {
    Single(&'static str, &'static str, &'queue [SingleReset]),
    Linked(&'static str, &'queue [LinkedWrite]),
    Window(&'queue [WindowAdvance]),
}

async fn commit_groups(
    store: &PostgresStore,
    groups: &[WriteGroup<'_>],
) -> Result<(), CommitError> {
    let mut client = store.session().await.map_err(CommitError::Statement)?;
    let transaction = client
        .transaction()
        .await
        .map_err(|source| CommitError::Statement(backend(source)))?;
    for group in groups {
        match group {
            WriteGroup::Single(table, column, queued) => {
                execute_queued(queued, async |reset| {
                    apply_single_reset(&transaction, table, column, reset).await
                })
                .await?;
            }
            WriteGroup::Linked(table, queued) => {
                execute_queued(queued, async |write| {
                    apply_linked_write(&transaction, table, write).await
                })
                .await?;
            }
            WriteGroup::Window(queued) => {
                execute_queued(queued, async |advance| {
                    apply_window_advance(&transaction, advance).await
                })
                .await?;
            }
        }
    }
    transaction
        .commit()
        .await
        .map_err(|source| CommitError::Commit(backend(source)))
}

async fn execute_queued<T>(
    queued: &[T],
    apply: impl AsyncFn(&T) -> Result<(), CommitError>,
) -> Result<(), CommitError>
where
    T: Sync,
{
    for item in queued {
        apply(item).await?;
    }
    Ok(())
}

async fn apply_window_advance<C>(client: &C, advance: &WindowAdvance) -> Result<(), CommitError>
where
    C: GenericClient + Sync,
{
    let budget_reset_at: NaiveDateTime = advance.budget_reset_at.naive_utc();
    client
        .execute(
            "UPDATE \"LiteLLM_BudgetTable\" SET budget_reset_at = $1 WHERE budget_id = $2",
            &[&budget_reset_at, &advance.budget_id],
        )
        .await
        .map_err(|source| CommitError::Statement(backend(source)))?;
    Ok(())
}

async fn apply_single_reset<C>(
    client: &C,
    table: &'static str,
    column: &'static str,
    reset: &SingleReset,
) -> Result<(), CommitError>
where
    C: GenericClient + Sync,
{
    let spend_decrement = to_f64(reset.data.spend.decrement)?;
    let budget_reset_at = reset.data.budget_reset_at.map(|value| value.naive_utc());
    let sql = format!(
        "UPDATE \"{table}\" SET spend = spend - $1, budget_reset_at = $2 WHERE {column} = $3 RETURNING {column}"
    );
    run_statement(
        client,
        sql.as_str(),
        &[&spend_decrement, &budget_reset_at, &reset.identifier],
    )
    .await?
    .ok_or(CommitError::MissingResetTarget { table })?;
    Ok(())
}

fn render_filter(
    filter: &LinkedResetFilter,
) -> Result<(String, Vec<Box<dyn ToSql + Sync + Send>>), CommitError> {
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
            params.push(Box::new(to_f64(bound)?));
            clauses.push(format!("spend > ${}", params.len()));
        }
        if let Some(bound) = range.less_than_or_equal {
            params.push(Box::new(to_f64(bound)?));
            clauses.push(format!("spend <= ${}", params.len()));
        }
    }
    Ok((clauses.join(" AND "), params))
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
            let (where_sql, params) = render_filter(filter)?;
            let sql = format!("UPDATE \"{table}\" SET spend = 0 WHERE {where_sql}");
            run_statement(client, sql.as_str(), &bind_params(&params)).await?;
        }
        LinkedWrite::Decrement(filter, amount) => {
            let decrement = to_f64(*amount)?;
            let (where_sql, mut params) = render_filter(filter)?;
            let amount_index = params.len().saturating_add(1);
            params.push(Box::new(decrement));
            let sql =
                format!("UPDATE \"{table}\" SET spend = spend - ${amount_index} WHERE {where_sql}");
            run_statement(client, sql.as_str(), &bind_params(&params)).await?;
        }
    }
    Ok(())
}
