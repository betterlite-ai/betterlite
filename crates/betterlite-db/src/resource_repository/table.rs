mod query;
mod sql;

use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_redis::config_sync::table_actions::TableActions;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio_postgres::types::ToSql;

use crate::auxiliary_repository::repository::{TableAccess, TableBinding, TableError};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;

use self::query::{execute_count, query_all, query_one};
use self::sql::{
    create_statement, delete_returning_statement, delete_statement, generate_id,
    select_one_statement, update_statement, upsert_statement,
};

pub trait DocumentData: Serialize {
    fn into_document(self) -> Result<BTreeMap<String, JsonData>, TableError>
    where
        Self: Sized,
    {
        let value = serde_json::to_value(self).map_err(TableError::Row)?;
        serde_json::from_value(value).map_err(TableError::Row)
    }
}

// Every live Prisma table needs the same actions, differing only in key, generated id and payload.
pub trait ResourceBinding: TableBinding {
    const KEY: &'static str;
    const ID: Option<&'static str>;
    const UPDATED_AT: Option<&'static str>;
    type Create: DocumentData + Send + 'static;
    type Update: DocumentData + Send + 'static;
}

// LiteLLM's table actions take a mapping and pass every key through, unknown ones included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RowWrite(pub BTreeMap<String, JsonData>);

impl DocumentData for RowWrite {
    fn into_document(self) -> Result<BTreeMap<String, JsonData>, TableError> {
        Ok(self.0)
    }
}

pub struct ResourceActions<'db, B: ResourceBinding> {
    access: TableAccess<'db>,
    binding: PhantomData<B>,
}

impl<'db, B: ResourceBinding + Sync> ResourceActions<'db, B> {
    #[must_use]
    pub const fn new(access: TableAccess<'db>) -> Self {
        Self {
            access,
            binding: PhantomData,
        }
    }

    pub async fn find_unique<R>(&self, key: &str) -> Result<Option<R>, TableError>
    where
        R: DeserializeOwned,
    {
        let sql = select_one_statement::<B>();
        let params: [&(dyn ToSql + Sync); 1] = [&key];
        query_one(self.access.read, &sql, &params).await
    }

    pub async fn find_many<R>(&self) -> Result<Vec<R>, TableError>
    where
        R: DeserializeOwned,
    {
        query_all(
            self.access.read,
            &format!("SELECT row_to_json(t)::text FROM \"{}\" AS t", B::TABLE),
            &[],
        )
        .await
    }

    pub async fn find_excluding<R>(&self, excluded: &str) -> Result<Vec<R>, TableError>
    where
        R: DeserializeOwned,
    {
        let mut sql = String::from("SELECT row_to_json(t)::text FROM \"");
        sql.push_str(B::TABLE);
        sql.push_str("\" AS t WHERE t.\"");
        sql.push_str(B::KEY);
        sql.push_str("\" <> $1");
        query_all(self.access.read, &sql, &[&excluded]).await
    }
}

impl<B: ResourceBinding + Send + Sync> TableActions for ResourceActions<'_, B> {
    type Row = JsonData;
    type Filter = String;
    type CreateData = B::Create;
    type UpdateData = B::Update;
    type Failure = TableError;

    async fn create(&self, data: Self::CreateData) -> Result<Self::Row, Self::Failure> {
        let mut record = data.into_document()?;
        generate_id::<B>(&mut record);
        let sql = create_statement::<B>(&record);
        let json = serde_json::to_string(&record).map_err(TableError::Row)?;
        let params: &[&(dyn ToSql + Sync)] = if record.is_empty() { &[] } else { &[&json] };
        query_one(self.access.write, &sql, params)
            .await?
            .ok_or(TableError::MissingRow)
    }

    async fn create_many(&self, data: Vec<Self::CreateData>) -> Result<usize, Self::Failure> {
        let total = data.len();
        let mut client = self.access.write.session().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        for item in data {
            let mut record = item.into_document()?;
            generate_id::<B>(&mut record);
            let sql = create_statement::<B>(&record);
            let json = serde_json::to_string(&record).map_err(TableError::Row)?;
            let params: &[&(dyn ToSql + Sync)] = if record.is_empty() { &[] } else { &[&json] };
            transaction
                .execute(&sql, params)
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
        }
        transaction
            .commit()
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(total)
    }

    async fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<Option<Self::Row>, Self::Failure> {
        let record = data.into_document()?;
        let sql = update_statement::<B>(&record);
        let json = serde_json::to_string(&record).map_err(TableError::Row)?;
        query_one(self.access.write, &sql, &[&json, &filter]).await
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        let record = data.into_document()?;
        let sql = update_statement::<B>(&record);
        let json = serde_json::to_string(&record).map_err(TableError::Row)?;
        execute_count(self.access.write, &sql, &[&json, &filter]).await
    }

    async fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        let mut record = data.into_document()?;
        record.insert(B::KEY.to_owned(), JsonData::String(filter));
        generate_id::<B>(&mut record);
        let sql = upsert_statement::<B>(&record);
        let json = serde_json::to_string(&record).map_err(TableError::Row)?;
        query_one(self.access.write, &sql, &[&json])
            .await?
            .ok_or(TableError::MissingRow)
    }

    async fn delete(&self, filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        let sql = delete_returning_statement::<B>();
        query_one(self.access.write, &sql, &[&filter]).await
    }

    async fn delete_many(&self, filter: Self::Filter) -> Result<usize, Self::Failure> {
        let sql = delete_statement::<B>();
        execute_count(self.access.write, &sql, &[&filter]).await
    }
}
