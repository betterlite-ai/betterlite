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

use self::query::{execute_count, query_all};
use self::sql::{
    DELETE, RETURNING, SELECT, assignments, generate_id, insert_statement, keyed_statement,
    update_statement,
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

// The live reads take every row, the row with one key, or every row but the one with that key.
#[derive(Debug, Clone, Copy)]
pub enum KeyFilter<'key> {
    All,
    Equal(&'key str),
    NotEqual(&'key str),
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

    pub async fn find<R>(&self, filter: KeyFilter<'_>) -> Result<Vec<R>, TableError>
    where
        R: DeserializeOwned,
    {
        let (comparison, key) = match filter {
            KeyFilter::All => {
                let every_row = format!("{SELECT} \"{}\" AS t", B::TABLE);
                return query_all(self.access.read, &every_row, &[]).await;
            }
            KeyFilter::Equal(matched) => ("=", matched),
            KeyFilter::NotEqual(skipped) => ("<>", skipped),
        };
        let sql = keyed_statement::<B>(SELECT, comparison, "");
        query_all(self.access.read, &sql, &[&key]).await
    }

    // update keeps the one matched row and update_many counts them, so both run this statement.
    async fn updated_rows(
        &self,
        data: B::Update,
        filter: &str,
    ) -> Result<Vec<JsonData>, TableError> {
        let record = data.into_document()?;
        let sql = update_statement::<B>(&record);
        let json = serde_json::to_string(&record).map_err(TableError::Row)?;
        query_all(self.access.write, &sql, &[&json, &filter]).await
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
        let sql = insert_statement::<B>(&record, "");
        let json = serde_json::to_string(&record).map_err(TableError::Row)?;
        let params: &[&(dyn ToSql + Sync)] = if record.is_empty() { &[] } else { &[&json] };
        query_all(self.access.write, &sql, params)
            .await?
            .pop()
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
            let sql = insert_statement::<B>(&record, "");
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
        Ok(self.updated_rows(data, &filter).await?.pop())
    }

    async fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> Result<usize, Self::Failure> {
        Ok(self.updated_rows(data, &filter).await?.len())
    }

    async fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> Result<Self::Row, Self::Failure> {
        let mut record = data.into_document()?;
        record.insert(B::KEY.to_owned(), JsonData::String(filter));
        generate_id::<B>(&mut record);
        let kept: Vec<&str> = [Some(B::KEY), B::ID].into_iter().flatten().collect();
        let conflict = format!(
            " ON CONFLICT (\"{}\") DO UPDATE SET {}",
            B::KEY,
            assignments::<B>(&record, "EXCLUDED", &kept)
        );
        let sql = insert_statement::<B>(&record, &conflict);
        let json = serde_json::to_string(&record).map_err(TableError::Row)?;
        query_all(self.access.write, &sql, &[&json])
            .await?
            .pop()
            .ok_or(TableError::MissingRow)
    }

    async fn delete(&self, filter: Self::Filter) -> Result<Option<Self::Row>, Self::Failure> {
        let sql = keyed_statement::<B>(DELETE, "=", RETURNING);
        Ok(query_all(self.access.write, &sql, &[&filter]).await?.pop())
    }

    async fn delete_many(&self, filter: Self::Filter) -> Result<usize, Self::Failure> {
        let sql = keyed_statement::<B>(DELETE, "=", "");
        execute_count(self.access.write, &sql, &[&filter]).await
    }
}
