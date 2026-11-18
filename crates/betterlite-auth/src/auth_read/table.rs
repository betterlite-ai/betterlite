use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LookupClock, SystemClock};
use tokio_util::task::TaskTracker;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence<T> {
    Omitted,
    Null,
    Value(T),
}

pub struct FindOne<Filter, Include> {
    pub filter: Filter,
    pub include: Presence<Include>,
}

pub struct FindMany<Filter, Include> {
    pub filter: Presence<Filter>,
    pub include: Presence<Include>,
    pub take: Presence<i64>,
}

pub struct Update<Filter, Data> {
    pub filter: Filter,
    pub data: Data,
}

pub struct Create<Data, Include> {
    pub data: Data,
    pub include: Presence<Include>,
}

#[async_trait]
pub trait AuthTable: Send + Sync + 'static {
    type Row: Send + 'static;
    type Filter: Send + 'static;
    type Include: Send + 'static;
    type Data: Send + 'static;

    async fn find_unique(
        &self,
        request: FindOne<Self::Filter, Self::Include>,
    ) -> Result<Option<Self::Row>, DbLookupError>;

    async fn find_first(
        &self,
        request: FindOne<Self::Filter, Self::Include>,
    ) -> Result<Option<Self::Row>, DbLookupError>;

    async fn find_many(
        &self,
        request: FindMany<Self::Filter, Self::Include>,
    ) -> Result<Vec<Self::Row>, DbLookupError>;

    async fn update(
        &self,
        request: Update<Self::Filter, Self::Data>,
    ) -> Result<Option<Self::Row>, DbLookupError>;

    async fn create(
        &self,
        request: Create<Self::Data, Self::Include>,
    ) -> Result<Self::Row, DbLookupError>;
}

pub struct ReadContext<Clock = SystemClock> {
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<Clock>>,
    pub tasks: TaskTracker,
}

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
    async fn bounded<Output, Lookup>(&self, lookup: Lookup) -> Result<Output, DbLookupError>
    where
        Output: Send + 'static,
        Lookup: Future<Output = Result<Output, DbLookupError>> + Send + 'static,
    {
        let request = LookupRequest {
            name: &self.lookup,
            deadline_seconds: None,
            default_deadline_seconds: self.context.default_deadline_seconds,
            tracker: self.context.tracker.as_ref(),
        };
        bounded_db_lookup(lookup, request, &self.context.tasks).await
    }
}

#[async_trait]
impl<T, Clock> AuthTable for BoundedTable<T, Clock>
where
    T: AuthTable,
    Clock: LookupClock + 'static,
{
    type Row = T::Row;
    type Filter = T::Filter;
    type Include = T::Include;
    type Data = T::Data;

    async fn find_unique(
        &self,
        request: FindOne<T::Filter, T::Include>,
    ) -> Result<Option<T::Row>, DbLookupError> {
        let table = Arc::clone(&self.table);
        self.bounded(async move { table.find_unique(request).await })
            .await
    }

    async fn find_first(
        &self,
        request: FindOne<T::Filter, T::Include>,
    ) -> Result<Option<T::Row>, DbLookupError> {
        let table = Arc::clone(&self.table);
        self.bounded(async move { table.find_first(request).await })
            .await
    }

    async fn find_many(
        &self,
        request: FindMany<T::Filter, T::Include>,
    ) -> Result<Vec<T::Row>, DbLookupError> {
        let table = Arc::clone(&self.table);
        self.bounded(async move { table.find_many(request).await })
            .await
    }

    async fn update(
        &self,
        request: Update<T::Filter, T::Data>,
    ) -> Result<Option<T::Row>, DbLookupError> {
        self.table.update(request).await
    }

    async fn create(&self, request: Create<T::Data, T::Include>) -> Result<T::Row, DbLookupError> {
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
    type Filter = T::Filter;
    type Include = T::Include;
    type Data = T::Data;

    async fn find_unique(
        &self,
        request: FindOne<T::Filter, T::Include>,
    ) -> Result<Option<T::Row>, DbLookupError> {
        match self {
            Self::Bounded(table) => table.find_unique(request).await,
            Self::Unbounded(table) => table.find_unique(request).await,
        }
    }

    async fn find_first(
        &self,
        request: FindOne<T::Filter, T::Include>,
    ) -> Result<Option<T::Row>, DbLookupError> {
        match self {
            Self::Bounded(table) => table.find_first(request).await,
            Self::Unbounded(table) => table.find_first(request).await,
        }
    }

    async fn find_many(
        &self,
        request: FindMany<T::Filter, T::Include>,
    ) -> Result<Vec<T::Row>, DbLookupError> {
        match self {
            Self::Bounded(table) => table.find_many(request).await,
            Self::Unbounded(table) => table.find_many(request).await,
        }
    }

    async fn update(
        &self,
        request: Update<T::Filter, T::Data>,
    ) -> Result<Option<T::Row>, DbLookupError> {
        match self {
            Self::Bounded(table) => table.update(request).await,
            Self::Unbounded(table) => table.update(request).await,
        }
    }

    async fn create(&self, request: Create<T::Data, T::Include>) -> Result<T::Row, DbLookupError> {
        match self {
            Self::Bounded(table) => table.create(request).await,
            Self::Unbounded(table) => table.create(request).await,
        }
    }
}
