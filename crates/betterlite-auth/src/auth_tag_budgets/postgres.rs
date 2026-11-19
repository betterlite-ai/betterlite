use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::database_failures::gate::SystemClock;
use betterlite_db::managed_resource_rows::tag::TagTable;
use deadpool_postgres::GenericClient;

use super::rows::group::ModelAccessGroupBudgetTable;
use super::rows::tag::TagBudgetTable;
use super::rows::{
    BudgetInclude, GroupRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure, TagRows,
};
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::repository::RepositoryTable;
use crate::auth_read::table::{
    AuthReadError, AuthReadTable, AuthTable, FindMany, Presence, ReadContext,
};
use crate::auth_resource_grants::rows::RowError;

// B726's tag table read through B2315's unwrapped adapter, so a batch read is one native query.
#[must_use]
pub fn open_tag_rows<Client>(
    client: Arc<Client>,
    context: ReadContext<SystemClock>,
) -> AuthReadTable<RepositoryTable<Client, TagBudgetTable>, SystemClock>
where
    RepositoryTable<Client, TagBudgetTable>: AuthTable,
{
    let table = Arc::new(RepositoryTable {
        client: Some(client),
        table: TagBudgetTable,
    });
    AuthReadAdapter::Tag.open(table, String::new(), context)
}

// B726's model access group budget table, read the same way.
#[must_use]
pub fn open_group_rows<Client>(
    client: Arc<Client>,
    context: ReadContext<SystemClock>,
) -> AuthReadTable<RepositoryTable<Client, ModelAccessGroupBudgetTable>, SystemClock>
where
    RepositoryTable<Client, ModelAccessGroupBudgetTable>: AuthTable,
{
    let table = Arc::new(RepositoryTable {
        client: Some(client),
        table: ModelAccessGroupBudgetTable,
    });
    AuthReadAdapter::ModelAccessGroupBudget.open(table, String::new(), context)
}

fn failure(source: AuthReadError<RowError>) -> TagBudgetRowsFailure {
    TagBudgetRowsFailure {
        source: Box::new(source),
    }
}

#[async_trait]
impl<Client> TagRows for AuthReadTable<RepositoryTable<Client, TagBudgetTable>, SystemClock>
where
    Client: GenericClient + Send + Sync + 'static,
{
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        let request = FindMany {
            filter: Presence::Value(names),
            include: Presence::Value(BudgetInclude {
                litellm_budget_table: true,
            }),
            take: Presence::Omitted,
        };
        self.find_many(request).await.map_err(failure)
    }

    async fn registry_rows(&self, take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        let request = FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Value(take),
        };
        self.find_many(request).await.map_err(failure)
    }
}

#[async_trait]
impl<Client> GroupRows
    for AuthReadTable<RepositoryTable<Client, ModelAccessGroupBudgetTable>, SystemClock>
where
    Client: GenericClient + Send + Sync + 'static,
{
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        let request = FindMany {
            filter: Presence::Value(names),
            include: Presence::Value(BudgetInclude {
                litellm_budget_table: true,
            }),
            take: Presence::Omitted,
        };
        self.find_many(request).await.map_err(failure)
    }

    async fn registry_rows(
        &self,
        take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        let request = FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Value(take),
        };
        self.find_many(request).await.map_err(failure)
    }
}
