use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LookupClock, SystemClock};
use betterlite_db::postgres_store::errors::DbError as RepositoryFault;
use tokio_util::task::TaskTracker;

// Both failure kinds cross the deadline boundary unchanged: LiteLLM's lookup fault or B700's typed error.
#[derive(Debug, thiserror::Error)]
pub enum AuthReadError<Failure>
where
    Failure: Error + 'static,
{
    #[error(transparent)]
    Lookup(DbLookupError),
    #[error(transparent)]
    Repository(RepositoryFault<Failure>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence<T> {
    Omitted,
    Null,
    Value(T),
}

impl<T> Presence<T> {
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Omitted | Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindOne<Key, Include> {
    pub filter: Key,
    pub include: Presence<Include>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindMany<Filter, Include> {
    pub filter: Presence<Filter>,
    pub include: Presence<Include>,
    pub take: Presence<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update<Key, Data> {
    pub filter: Key,
    pub data: Data,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Create<Data, Include> {
    pub data: Data,
    pub include: Presence<Include>,
}

// Native tables supply B700's key/filter/data/row types; Presence keeps Prisma's absent/null kwargs.
#[async_trait]
pub trait AuthTable: Send + Sync + 'static {
    type Row: Send + 'static;
    type Key: Send + 'static;
    type Filter: Send + 'static;
    type Include: Send + 'static;
    type Data: Send + 'static;
    type Failure: Error + Send + Sync + 'static;

    async fn find_unique(
        &self,
        request: FindOne<Self::Key, Self::Include>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>>;

    async fn find_first(
        &self,
        request: FindOne<Self::Filter, Self::Include>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>>;

    async fn find_many(
        &self,
        request: FindMany<Self::Filter, Self::Include>,
    ) -> Result<Vec<Self::Row>, AuthReadError<Self::Failure>>;

    async fn update(
        &self,
        request: Update<Self::Key, Self::Data>,
    ) -> Result<Option<Self::Row>, AuthReadError<Self::Failure>>;

    async fn create(
        &self,
        request: Create<Self::Data, Self::Include>,
    ) -> Result<Self::Row, AuthReadError<Self::Failure>>;
}

pub struct ReadContext<Clock = SystemClock> {
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<Clock>>,
    pub tasks: TaskTracker,
}

pub(super) type OneRequest<Table> =
    FindOne<<Table as AuthTable>::Key, <Table as AuthTable>::Include>;
pub(super) type FirstRequest<Table> =
    FindOne<<Table as AuthTable>::Filter, <Table as AuthTable>::Include>;
pub(super) type ManyRequest<Table> =
    FindMany<<Table as AuthTable>::Filter, <Table as AuthTable>::Include>;
pub(super) type UpdateRequest<Table> =
    Update<<Table as AuthTable>::Key, <Table as AuthTable>::Data>;
pub(super) type CreateRequest<Table> =
    Create<<Table as AuthTable>::Data, <Table as AuthTable>::Include>;
pub(super) type OneAnswer<Table> =
    Result<Option<<Table as AuthTable>::Row>, AuthReadError<<Table as AuthTable>::Failure>>;
pub(super) type ManyAnswer<Table> =
    Result<Vec<<Table as AuthTable>::Row>, AuthReadError<<Table as AuthTable>::Failure>>;
pub(super) type RowAnswer<Table> =
    Result<<Table as AuthTable>::Row, AuthReadError<<Table as AuthTable>::Failure>>;

pub struct BoundedTable<T, Clock = SystemClock> {
    pub(super) table: Arc<T>,
    pub(super) lookup: String,
    pub(super) context: ReadContext<Clock>,
}

impl<T, Clock> BoundedTable<T, Clock>
where
    T: AuthTable,
    Clock: LookupClock + 'static,
{
    async fn bounded<Answer, Lookup>(
        &self,
        lookup: Lookup,
    ) -> Result<Answer, AuthReadError<T::Failure>>
    where
        Answer: Send + 'static,
        Lookup: Future<Output = Result<Answer, AuthReadError<T::Failure>>> + Send + 'static,
    {
        let request = LookupRequest {
            name: &self.lookup,
            deadline_seconds: None,
            default_deadline_seconds: self.context.default_deadline_seconds,
            tracker: self.context.tracker.as_ref(),
        };
        let outcome = bounded_db_lookup(
            async move {
                match lookup.await {
                    Ok(answer) => Ok(Ok::<Answer, AuthReadError<T::Failure>>(answer)),
                    Err(AuthReadError::Lookup(failure)) => Err(failure),
                    Err(AuthReadError::Repository(fault)) => {
                        Ok(Err(AuthReadError::Repository(fault)))
                    }
                }
            },
            request,
            &self.context.tasks,
        )
        .await;
        match outcome {
            Ok(answer) => answer,
            Err(deadline) => Err(AuthReadError::Lookup(deadline)),
        }
    }
}

#[async_trait]
impl<T, Clock> AuthTable for BoundedTable<T, Clock>
where
    T: AuthTable,
    Clock: LookupClock + 'static,
{
    type Row = T::Row;
    type Key = T::Key;
    type Filter = T::Filter;
    type Include = T::Include;
    type Data = T::Data;
    type Failure = T::Failure;

    async fn find_unique(&self, request: OneRequest<T>) -> OneAnswer<T> {
        let table = Arc::clone(&self.table);
        self.bounded(async move { table.find_unique(request).await })
            .await
    }

    async fn find_first(&self, request: FirstRequest<T>) -> OneAnswer<T> {
        let table = Arc::clone(&self.table);
        self.bounded(async move { table.find_first(request).await })
            .await
    }

    async fn find_many(&self, request: ManyRequest<T>) -> ManyAnswer<T> {
        let table = Arc::clone(&self.table);
        self.bounded(async move { table.find_many(request).await })
            .await
    }

    async fn update(&self, request: UpdateRequest<T>) -> OneAnswer<T> {
        self.table.update(request).await
    }

    async fn create(&self, request: CreateRequest<T>) -> RowAnswer<T> {
        self.table.create(request).await
    }
}

pub enum AuthReadTable<T, Clock = SystemClock> {
    // boxed to stay within 3x of Unbounded, the limit variant_size_differences enforces
    Bounded(Box<BoundedTable<T, Clock>>),
    Unbounded(Arc<T>),
}

#[async_trait]
impl<T, Clock> AuthTable for AuthReadTable<T, Clock>
where
    T: AuthTable,
    Clock: LookupClock + 'static,
{
    type Row = T::Row;
    type Key = T::Key;
    type Filter = T::Filter;
    type Include = T::Include;
    type Data = T::Data;
    type Failure = T::Failure;

    async fn find_unique(&self, request: OneRequest<T>) -> OneAnswer<T> {
        match self {
            Self::Bounded(table) => table.find_unique(request).await,
            Self::Unbounded(table) => table.find_unique(request).await,
        }
    }

    async fn find_first(&self, request: FirstRequest<T>) -> OneAnswer<T> {
        match self {
            Self::Bounded(table) => table.find_first(request).await,
            Self::Unbounded(table) => table.find_first(request).await,
        }
    }

    async fn find_many(&self, request: ManyRequest<T>) -> ManyAnswer<T> {
        match self {
            Self::Bounded(table) => table.find_many(request).await,
            Self::Unbounded(table) => table.find_many(request).await,
        }
    }

    async fn update(&self, request: UpdateRequest<T>) -> OneAnswer<T> {
        match self {
            Self::Bounded(table) => table.update(request).await,
            Self::Unbounded(table) => table.update(request).await,
        }
    }

    async fn create(&self, request: CreateRequest<T>) -> RowAnswer<T> {
        match self {
            Self::Bounded(table) => table.create(request).await,
            Self::Unbounded(table) => table.create(request).await,
        }
    }
}
