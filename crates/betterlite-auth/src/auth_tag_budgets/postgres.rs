use async_trait::async_trait;
use betterlite_db::database_failures::gate::SystemClock;
use deadpool_postgres::GenericClient;

use super::rows::{BudgetInclude, BudgetRows, TagBudgetRowsFailure};
use crate::auth_read::repository::RepositoryTable;
use crate::auth_read::table::{AuthReadError, AuthReadTable, AuthTable, FindMany, Presence};
use crate::auth_resource_grants::rows::RowError;

fn failure(source: AuthReadError<RowError>) -> TagBudgetRowsFailure {
    TagBudgetRowsFailure {
        source: Box::new(source),
    }
}

fn names_request(names: Vec<String>) -> FindMany<Vec<String>, BudgetInclude> {
    FindMany {
        filter: Presence::Value(names),
        include: Presence::Value(BudgetInclude {
            litellm_budget_table: true,
        }),
        take: Presence::Omitted,
    }
}

fn take_request(take: i64) -> FindMany<Vec<String>, BudgetInclude> {
    FindMany {
        filter: Presence::Omitted,
        include: Presence::Omitted,
        take: Presence::Value(take),
    }
}

#[async_trait]
impl<Client, Rows, Row> BudgetRows<Row>
    for AuthReadTable<RepositoryTable<Client, Rows>, SystemClock>
where
    Client: GenericClient + Send + Sync + 'static,
    Rows: Send + Sync + 'static,
    RepositoryTable<Client, Rows>:
        AuthTable<Row = Row, Filter = Vec<String>, Include = BudgetInclude, Failure = RowError>,
{
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<Row>, TagBudgetRowsFailure> {
        self.find_many(names_request(names)).await.map_err(failure)
    }

    async fn registry_rows(&self, take: i64) -> Result<Vec<Row>, TagBudgetRowsFailure> {
        self.find_many(take_request(take)).await.map_err(failure)
    }
}
