use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::EndUserTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::budget::BudgetRepository;
use betterlite_db::resource_repository::table::KeyFilter;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use tokio_util::task::TaskTracker;

use super::rows::{BudgetFetchError, BudgetRows, EndUserFetchError, EndUserRowTable, EndUserRows};
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::table::{
    AuthReadError, AuthTable, Create, FindMany, FindOne, Presence, ReadContext, Update,
};

// B2315's bounded budget table over B725's BudgetRepository, which is LiteLLM's _dictable_table(..., "budget")
pub struct BudgetTable {
    pub writer: Arc<PostgresStore>,
    pub reader: Arc<PostgresStore>,
    pub availability: Arc<Availability>,
}

// only the unique read is part of the default-budget lookup
const UNREAD: AuthReadError<TableError> = AuthReadError::Repository(DbError::CreateNull);

#[async_trait]
impl AuthTable for BudgetTable {
    type Row = LiteLLMBudgetTable;
    type Key = String;
    type Filter = String;
    type Include = ();
    type Data = ();
    type Failure = TableError;

    async fn find_unique(
        &self,
        request: FindOne<String, ()>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>> {
        let client = PrismaClient {
            writer: self.writer.as_ref(),
            reader: self.reader.as_ref(),
            availability: self.availability.as_ref(),
        };
        let actions = BudgetRepository::new(Some(client))
            .table()
            .map_err(|source| AuthReadError::Repository(widen(source)))?;
        match actions
            .as_ref()
            .find::<LiteLLMBudgetTable>(KeyFilter::Equal(request.filter.as_str()))
            .await
        {
            Ok(mut rows) => Ok(rows.pop()),
            Err(source) => Err(AuthReadError::Repository(fault(source))),
        }
    }

    async fn find_first(
        &self,
        _request: FindOne<String, ()>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>> {
        Err(UNREAD)
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

// The live end-user and budget reads a production lookup is built on, routed by B714
pub struct DatabaseEndUsers {
    pub writer: Arc<PostgresStore>,
    pub reader: Arc<PostgresStore>,
    pub availability: Arc<Availability>,
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
}

#[async_trait]
impl EndUserRows for DatabaseEndUsers {
    async fn find_end_user(
        &self,
        end_user_id: &str,
    ) -> Result<Option<LiteLLMEndUserTable>, EndUserFetchError> {
        let client = PrismaClient {
            writer: self.writer.as_ref(),
            reader: self.reader.as_ref(),
            availability: self.availability.as_ref(),
        };
        let repository = PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            EndUserTable,
        );
        let access = repository.access().map_err(|source| {
            EndUserFetchError::Read(Box::new(AuthReadError::Repository(widen(source))))
        })?;
        let session = access.read.session().await.map_err(|source| {
            EndUserFetchError::Read(Box::new(AuthReadError::Repository(widen(source))))
        })?;
        let table = Arc::new(EndUserRowTable {
            client: Some(session),
        });
        let context = ReadContext {
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        };
        let bounded = AuthReadAdapter::Dictable.open(table, "end_user".to_owned(), context);
        bounded
            .find_unique(FindOne {
                filter: end_user_id.to_owned(),
                include: Presence::Omitted,
            })
            .await
            .map_err(|source| EndUserFetchError::Read(Box::new(source)))
    }
}

#[async_trait]
impl BudgetRows for DatabaseEndUsers {
    async fn find_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, BudgetFetchError> {
        let table = Arc::new(BudgetTable {
            writer: Arc::clone(&self.writer),
            reader: Arc::clone(&self.reader),
            availability: Arc::clone(&self.availability),
        });
        let context = ReadContext {
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        };
        let bounded = AuthReadAdapter::Dictable.open(table, "budget".to_owned(), context);
        bounded
            .find_unique(FindOne {
                filter: budget_id.to_owned(),
                include: Presence::Omitted,
            })
            .await
            .map_err(classify)
    }
}

// a row the model rejects is LiteLLM's decode outside the try block; everything else is the query
fn classify(error: AuthReadError<TableError>) -> BudgetFetchError {
    let decoded = matches!(
        &error,
        AuthReadError::Repository(DbError::InvalidRecord {
            source: TableError::Row(_),
        })
    );
    let boxed = Box::new(error);
    if decoded {
        return BudgetFetchError::Decode(boxed);
    }
    BudgetFetchError::Query(boxed)
}

// BudgetRepository::table builds DbError<Infallible>, so only the source-free variants pass through
fn widen<Failure>(error: DbError) -> DbError<Failure> {
    match error {
        DbError::NoDbConnected => DbError::NoDbConnected,
        DbError::Pool { source } => DbError::Pool { source },
        DbError::Backend { source } => DbError::Backend { source },
        DbError::InvalidRecord { source } => match source {},
        DbError::CreateNull => DbError::CreateNull,
        DbError::UniqueViolation { constraint, source } => {
            DbError::UniqueViolation { constraint, source }
        }
        DbError::SqlState {
            code,
            constraint,
            source,
        } => DbError::SqlState {
            code,
            constraint,
            source,
        },
    }
}

fn fault(error: TableError) -> DbError<TableError> {
    if let TableError::Database(source) = error {
        return widen(source);
    }
    DbError::InvalidRecord { source: error }
}
