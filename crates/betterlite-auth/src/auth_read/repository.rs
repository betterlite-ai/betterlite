use std::slice::from_ref;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::repository::Repository;
use betterlite_db::postgres_store::rows::{FindMany as RowsFindMany, Key, KeyAction, KeyColumn};
use deadpool_postgres::GenericClient;
use tokio_postgres::types::ToSql;

use super::include::{Including, SelectingTable};
use super::table::{
    AuthReadError, AuthTable, CreateRequest, FirstRequest, ManyAnswer, ManyRequest, OneAnswer,
    OneRequest, RowAnswer, UpdateRequest,
};

// Owned form of B700's borrowed Key; the box lets an owned AuthTable::Key outlive the query build.
pub struct NativeKey<Column> {
    pub column: KeyColumn<Column>,
    pub value: Box<dyn ToSql + Send + Sync>,
}

// B700's Repository over an owned table, so auth reads use the native tables and row decoders.
pub struct RepositoryTable<Client, Rows> {
    pub client: Option<Arc<Client>>,
    pub table: Rows,
}

// The read a caller asked for, so the adapter reaches B700's repository through one function.
enum ReadQuery<'query, Rows: Including> {
    Keyed(
        KeyAction<'query, Rows::Data>,
        &'query NativeKey<Rows::Column>,
        Option<&'query Rows::Include>,
    ),
    Listed(
        Option<&'query Rows::Filter>,
        Option<i64>,
        Option<&'query Rows::Include>,
    ),
}

impl<'query, Rows: Including> ReadQuery<'query, Rows> {
    const fn include(&self) -> Option<&'query Rows::Include> {
        match self {
            Self::Keyed(_, _, include) | Self::Listed(_, _, include) => *include,
        }
    }

    // B700 hands take by value, so the selecting table owns the limit and binds it into the statement.
    const fn take(&self) -> Option<i64> {
        match self {
            Self::Keyed(..) => None,
            Self::Listed(_, take, _) => *take,
        }
    }
}

impl<Client, Rows> RepositoryTable<Client, Rows>
where
    Client: GenericClient + Sync,
    Rows: Including + Clone + Sync,
    Rows::Column: Copy + Send + Sync,
    Rows::Data: Sync,
    Rows::Filter: Sync,
    Rows::Order: Sync,
{
    async fn read(
        &self,
        query: ReadQuery<'_, Rows>,
    ) -> Result<Vec<Rows::Model>, AuthReadError<Rows::Error>> {
        let repository = Repository {
            client: self.client.as_deref(),
            table: SelectingTable {
                table: self.table.clone(),
                include: query.include().cloned(),
                limit: query.take(),
            },
        };
        match query {
            ReadQuery::Keyed(action, filter, _) => {
                let key = Key {
                    column: filter.column,
                    value: &*filter.value,
                };
                let found = repository
                    .by_key(action, key)
                    .await
                    .map_err(AuthReadError::Repository)?;
                Ok(found.into_iter().collect())
            }
            ReadQuery::Listed(filter, _, _) => {
                let options = RowsFindMany {
                    filter: filter.map(from_ref),
                    order: None,
                    skip: None,
                    take: None,
                };
                repository
                    .find_many(options)
                    .await
                    .map_err(AuthReadError::Repository)
            }
        }
    }

    async fn created(
        &self,
        data: &Rows::Data,
        include: Option<&Rows::Include>,
    ) -> Result<Rows::Model, AuthReadError<Rows::Error>> {
        let repository = Repository {
            client: self.client.as_deref(),
            table: SelectingTable {
                table: self.table.clone(),
                include: include.cloned(),
                limit: None,
            },
        };
        repository
            .create(data)
            .await
            .map_err(AuthReadError::Repository)
    }
}

#[async_trait]
impl<Client, Rows> AuthTable for RepositoryTable<Client, Rows>
where
    Client: GenericClient + Send + Sync + 'static,
    Rows: Including + Clone + Send + Sync + 'static,
    Rows::Model: Send + 'static,
    Rows::Column: Copy + Send + Sync + 'static,
    Rows::Data: Send + Sync + 'static,
    Rows::Filter: Send + Sync + 'static,
    Rows::Order: Send + Sync + 'static,
{
    type Row = Rows::Model;
    type Key = NativeKey<Rows::Column>;
    type Filter = Rows::Filter;
    type Include = Rows::Include;
    type Data = Rows::Data;
    type Failure = Rows::Error;

    async fn find_unique(&self, request: OneRequest<Self>) -> OneAnswer<Self> {
        let include = request.include.value();
        self.read(ReadQuery::Keyed(KeyAction::Find, &request.filter, include))
            .await
            .map(|found| found.into_iter().next())
    }

    async fn find_first(&self, request: FirstRequest<Self>) -> OneAnswer<Self> {
        let include = request.include.value();
        self.read(ReadQuery::Listed(Some(&request.filter), Some(1), include))
            .await
            .map(|found| found.into_iter().next())
    }

    async fn find_many(&self, request: ManyRequest<Self>) -> ManyAnswer<Self> {
        let filter = request.filter.value();
        let take = request.take.value().copied();
        let include = request.include.value();
        self.read(ReadQuery::Listed(filter, take, include)).await
    }

    async fn update(&self, request: UpdateRequest<Self>) -> OneAnswer<Self> {
        let action = KeyAction::Update(&request.data);
        self.read(ReadQuery::Keyed(action, &request.filter, None))
            .await
            .map(|found| found.into_iter().next())
    }

    async fn create(&self, request: CreateRequest<Self>) -> RowAnswer<Self> {
        self.created(&request.data, request.include.value()).await
    }
}
