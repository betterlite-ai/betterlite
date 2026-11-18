use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::DbError as LookupFault;
use betterlite_db::database_faults::ErrorType;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::postgres_store::errors::DbError as RepositoryFault;
use betterlite_db::postgres_store::repository::Repository;
use betterlite_db::postgres_store::rows::{
    FindMany as RowsFindMany, Key, KeyAction, KeyColumn, Table,
};
use deadpool_postgres::GenericClient;
use tokio_postgres::types::ToSql;

use super::table::{
    AuthTable, CreateRequest, ManyAnswer, ManyRequest, OneAnswer, OneRequest, RowAnswer,
    UpdateRequest,
};

// Owned form of B700's borrowed Key; the box lets an owned AuthTable::Key outlive the query build.
pub struct NativeKey<Column> {
    column: KeyColumn<Column>,
    value: Box<dyn ToSql + Send + Sync>,
}

impl<Column> NativeKey<Column> {
    #[must_use]
    pub fn new<Value>(column: KeyColumn<Column>, value: Value) -> Self
    where
        Value: ToSql + Send + Sync + 'static,
    {
        Self {
            column,
            value: Box::new(value),
        }
    }
}

impl<Column: Copy> NativeKey<Column> {
    fn key(&self) -> Key<'_, Column> {
        Key {
            column: self.column,
            value: &*self.value,
        }
    }
}

// B700's Repository over an owned table, so auth reads use the native tables and row decoders.
pub struct RepositoryTable<Client, Rows> {
    pub client: Option<Arc<Client>>,
    pub table: Rows,
}

impl<Client, Rows: Clone> RepositoryTable<Client, Rows> {
    fn repository(&self) -> Repository<'_, Client, Rows> {
        Repository {
            client: self.client.as_deref(),
            table: self.table.clone(),
        }
    }
}

impl<Client, Rows> RepositoryTable<Client, Rows>
where
    Client: GenericClient + Sync,
    Rows: Table + Clone + Sync,
    Rows::Column: Copy + Send + Sync,
    Rows::Data: Sync,
    Rows::Filter: Sync,
    Rows::Order: Sync,
{
    async fn find_keyed(
        &self,
        key: &NativeKey<Rows::Column>,
    ) -> Result<Option<Rows::Model>, DbLookupError> {
        let repository = self.repository();
        repository
            .by_key(KeyAction::Find, key.key())
            .await
            .map_err(|error| repository_fault(&error))
    }
}

#[async_trait]
impl<Client, Rows> AuthTable for RepositoryTable<Client, Rows>
where
    Client: GenericClient + Send + Sync + 'static,
    Rows: Table + Clone + Send + Sync + 'static,
    Rows::Model: Send + 'static,
    Rows::Column: Copy + Send + Sync + 'static,
    Rows::Data: Send + Sync + 'static,
    Rows::Filter: Send + Sync + 'static,
    Rows::Order: Send + Sync + 'static,
{
    type Row = Rows::Model;
    type Key = NativeKey<Rows::Column>;
    type Filter = Vec<Rows::Filter>;
    type Include = ();
    type Data = Rows::Data;

    async fn find_unique(&self, request: OneRequest<Self>) -> OneAnswer<Self> {
        self.find_keyed(&request.filter).await
    }

    async fn find_first(&self, request: OneRequest<Self>) -> OneAnswer<Self> {
        self.find_keyed(&request.filter).await
    }

    async fn find_many(&self, request: ManyRequest<Self>) -> ManyAnswer<Self> {
        let repository = self.repository();
        let options = RowsFindMany {
            filter: request.filter.value().map(Vec::as_slice),
            order: None,
            skip: None,
            take: request.take.value().copied(),
        };
        repository
            .find_many(options)
            .await
            .map_err(|error| repository_fault(&error))
    }

    async fn update(&self, request: UpdateRequest<Self>) -> OneAnswer<Self> {
        let repository = self.repository();
        repository
            .by_key(KeyAction::Update(&request.data), request.filter.key())
            .await
            .map_err(|error| repository_fault(&error))
    }

    async fn create(&self, request: CreateRequest<Self>) -> RowAnswer<Self> {
        let repository = self.repository();
        repository
            .create(&request.data)
            .await
            .map_err(|error| repository_fault(&error))
    }
}

// B700's error becomes a LiteLLM-shaped fault; its redacted Display keeps parameter values out.
fn repository_fault<Failure>(error: &RepositoryFault<Failure>) -> DbLookupError
where
    Failure: Error + 'static,
{
    let error_type = if is_native_transport_error(error) {
        ErrorType::Connect
    } else {
        ErrorType::Other
    };
    DbLookupError::Database(LookupFault {
        error_type,
        message: error.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    })
}
