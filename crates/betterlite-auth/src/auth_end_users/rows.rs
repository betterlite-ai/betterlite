use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::errors::{DbError, backend};
use tokio_postgres::Row;

use crate::auth_read::table::{AuthReadError, AuthTable, Create, FindMany, FindOne, Update};

// LiteLLM's find_unique for EndUserRepository with both Prisma includes beside the row.
pub const END_USER_ONE_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END, 'object_permission', CASE WHEN permission.\"object_permission_id\" IS NULL THEN NULL ELSE to_jsonb(permission) END))::text FROM \"LiteLLM_EndUserTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" LEFT JOIN \"LiteLLM_ObjectPermissionTable\" AS permission ON permission.\"object_permission_id\" = entry.\"object_permission_id\" WHERE entry.\"user_id\" = $1";

#[derive(Debug, thiserror::Error)]
pub enum EndUserRowError {
    #[error("stored end-user row column could not be read")]
    Column(#[source] DriverError),
    #[error("stored end-user row is not a readable record")]
    Record(#[source] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum EndUserFetchError {
    #[error("the end-user row lookup failed")]
    Read(#[source] Box<AuthReadError<EndUserRowError>>),
}

// LiteLLM catches only the query; a row its model rejects is raised after the try block
#[derive(Debug, thiserror::Error)]
pub enum BudgetFetchError {
    #[error("the budget row query failed")]
    Query(#[source] Box<AuthReadError<TableError>>),
    #[error("the stored budget row is not readable")]
    Decode(#[source] Box<AuthReadError<TableError>>),
}

// The end-user row a lookup reads; the database adapter and the tests both answer it.
#[async_trait]
pub trait EndUserRows: Send + Sync {
    async fn find_end_user(
        &self,
        end_user_id: &str,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserFetchError>;
}

// The budget rows both default-budget reads load by id.
#[async_trait]
pub trait BudgetRows: Send + Sync {
    async fn find_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, BudgetFetchError>;
}

// B2315's unbounded end-user table: Prisma's find_unique with the two include relations baked in.
pub struct EndUserRowTable {
    pub client: Option<deadpool_postgres::Client>,
}

// only the unique read is part of the lookup, so every other action answers a repository fault
const UNREAD: AuthReadError<EndUserRowError> = AuthReadError::Repository(DbError::CreateNull);

impl EndUserRowTable {
    async fn one(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMEndUserTable>, AuthReadError<EndUserRowError>> {
        let Some(client) = self.client.as_ref() else {
            return Err(AuthReadError::Repository(DbError::NoDbConnected));
        };
        let row = client
            .query_opt(END_USER_ONE_SQL, &[&user_id])
            .await
            .map_err(|source| AuthReadError::Repository(backend(source)))?;
        let Some(row) = row else {
            return Ok(None);
        };
        decode(&row)
    }
}

fn decode(row: &Row) -> Result<Option<LiteLLMEndUserTable>, AuthReadError<EndUserRowError>> {
    let text: String = row.try_get(0).map_err(|driver| {
        AuthReadError::Repository(DbError::InvalidRecord {
            source: EndUserRowError::Column(DriverError { driver }),
        })
    })?;
    serde_json::from_str(&text).map(Some).map_err(|source| {
        AuthReadError::Repository(DbError::InvalidRecord {
            source: EndUserRowError::Record(source),
        })
    })
}

#[async_trait]
impl AuthTable for EndUserRowTable {
    type Row = LiteLLMEndUserTable;
    type Key = String;
    type Filter = String;
    type Include = ();
    type Data = ();
    type Failure = EndUserRowError;

    async fn find_unique(
        &self,
        request: FindOne<String, ()>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>> {
        self.one(&request.filter).await
    }

    async fn find_first(
        &self,
        request: FindOne<String, ()>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>> {
        self.one(&request.filter).await
    }

    async fn find_many(
        &self,
        _request: FindMany<String, ()>,
    ) -> Result<Vec<Self::Row>, AuthReadError<Self::Failure>> {
        Err(UNREAD)
    }

    async fn update(
        &self,
        _request: Update<String, ()>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>> {
        Err(UNREAD)
    }

    async fn create(
        &self,
        _request: Create<(), ()>,
    ) -> Result<Self::Row, AuthReadError<Self::Failure>> {
        Err(UNREAD)
    }
}
