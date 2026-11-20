use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{LookupClock, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::permission::ObjectPermissionRepository;
use betterlite_db::resource_repository::table::KeyFilter;

use super::lookup::PermissionError;
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::table::{
    AuthReadError, AuthTable, Create, FindMany, FindOne, Presence, ReadContext, Update,
};

type OneAnswer = Result<Option<JsonData>, AuthReadError<TableError>>;
type ManyAnswer = Result<Vec<JsonData>, AuthReadError<TableError>>;
type RowAnswer = Result<JsonData, AuthReadError<TableError>>;
// dictable_table(ObjectPermissionRepository(...), "object_permission").find_unique(where={...})
#[async_trait]
pub trait ObjectPermissionStore: Send + Sync {
    async fn find_permission(&self, id: &str) -> Result<Option<JsonData>, PermissionError>;
}
// the live table's unique read over B726's repository, routed by B714 unless the writer is asked for
pub struct PermissionTable {
    pub reader: PostgresStore,
    pub writer: PostgresStore,
    pub availability: Arc<Availability>,
    pub use_writer: bool,
}
// the bounded permission lookup applies the label "object_permission" to the deadline and tracker
pub struct BoundedPermissionStore<T, Clock = SystemClock> {
    pub table: Arc<T>,
    pub context: ReadContext<Clock>,
}
#[async_trait]
impl<T, Clock> ObjectPermissionStore for BoundedPermissionStore<T, Clock>
where
    T: AuthTable<
            Row = JsonData,
            Key = String,
            Filter = String,
            Include = (),
            Data = (),
            Failure = TableError,
        >,
    Clock: LookupClock + 'static,
{
    async fn find_permission(&self, id: &str) -> Result<Option<JsonData>, PermissionError> {
        let context = ReadContext {
            default_deadline_seconds: self.context.default_deadline_seconds,
            tracker: Arc::clone(&self.context.tracker),
            tasks: self.context.tasks.clone(),
        };
        let table =
            AuthReadAdapter::ObjectPermission.open(Arc::clone(&self.table), String::new(), context);
        match table
            .find_unique(FindOne {
                filter: id.to_owned(),
                include: Presence::Omitted,
            })
            .await
        {
            Ok(row) => Ok(row),
            Err(AuthReadError::Lookup(error)) => Err(PermissionError::Lookup(error)),
            Err(AuthReadError::Repository(error)) => Err(PermissionError::Repository(error)),
        }
    }
}
// only the unique read is part of the permission lookup, so the other actions answer a fault
const UNREAD: AuthReadError<TableError> = AuthReadError::Repository(DbError::InvalidRecord {
    source: TableError::MissingRow,
});
#[async_trait]
impl AuthTable for PermissionTable {
    type Row = JsonData;
    type Key = String;
    type Filter = String;
    type Include = ();
    type Data = ();
    type Failure = TableError;
    async fn find_unique(&self, request: FindOne<String, ()>) -> OneAnswer {
        let client = PrismaClient {
            writer: &self.writer,
            reader: &self.reader,
            availability: self.availability.as_ref(),
        };
        let actions = ObjectPermissionRepository::new(Some(client), self.use_writer)
            .table()
            .map_err(|source| AuthReadError::Repository(widen(source)))?;
        match actions
            .as_ref()
            .find::<JsonData>(KeyFilter::Equal(request.filter.as_str()))
            .await
        {
            Ok(mut rows) => Ok(rows.pop()),
            Err(source) => Err(AuthReadError::Repository(fault(source))),
        }
    }
    async fn find_first(&self, _request: FindOne<String, ()>) -> OneAnswer {
        Err(UNREAD)
    }
    async fn find_many(&self, _request: FindMany<String, ()>) -> ManyAnswer {
        Err(UNREAD)
    }
    async fn update(&self, _request: Update<String, ()>) -> OneAnswer {
        Err(UNREAD)
    }
    async fn create(&self, _request: Create<(), ()>) -> RowAnswer {
        Err(UNREAD)
    }
}
// ObjectPermissionRepository::table builds DbError<Infallible>, so only the source-free variants pass through
fn widen(error: DbError) -> DbError<TableError> {
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
