use super::errors::{DbError, backend};
use super::rows::{FindMany, Key, KeyAction, KeyColumn, Query, Table};
use deadpool_postgres::GenericClient;
use std::collections::HashSet;
use std::hash::Hash;
use tokio_postgres::{Row, types::ToSql};

pub struct Repository<'client, C, T> {
    pub client: Option<&'client C>,
    pub table: T,
}

impl<C, T> Repository<'_, C, T>
where
    C: GenericClient + Sync,
    T: Table + Sync,
    T::Column: Send + Sync,
    T::Data: Sync,
    T::Filter: Sync,
    T::Order: Sync,
{
    async fn optional(&self, query: Query<'_>) -> Result<Option<Row>, DbError<T::Error>> {
        let client = self.client.ok_or(DbError::NoDbConnected)?;
        client
            .query_opt(query.sql, &query.parameters)
            .await
            .map_err(backend)
    }

    pub async fn by_key(
        &self,
        action: KeyAction<'_, T::Data>,
        key: Key<'_, T::Column>,
    ) -> Result<Option<T::Model>, DbError<T::Error>> {
        let row = self.optional(self.table.keyed(action, key)).await?;
        row.as_ref()
            .map(|record| self.table.decode(record))
            .transpose()
            .map(Option::flatten)
            .map_err(|source| DbError::InvalidRecord { source })
    }

    pub async fn create(&self, data: &T::Data) -> Result<T::Model, DbError<T::Error>> {
        let row = self
            .optional(self.table.insert(data))
            .await?
            .ok_or(DbError::CreateNull)?;
        self.table
            .decode(&row)
            .map_err(|source| DbError::InvalidRecord { source })?
            .ok_or(DbError::CreateNull)
    }

    pub async fn exists(&self, key: Key<'_, T::Column>) -> Result<bool, DbError<T::Error>> {
        let row = self
            .optional(self.table.keyed(KeyAction::Find, key))
            .await?;
        Ok(row.is_some())
    }

    pub async fn count(&self, filter: Option<&[T::Filter]>) -> Result<i64, DbError<T::Error>> {
        let query = self.table.count(filter);
        let client = self.client.ok_or(DbError::NoDbConnected)?;
        client
            .query_one(query.sql, &query.parameters)
            .await
            .map_err(backend)?
            .try_get(0)
            .map_err(backend)
    }

    pub async fn find_many(
        &self,
        mut options: FindMany<'_, T::Filter, T::Order>,
    ) -> Result<Vec<T::Model>, DbError<T::Error>> {
        options.filter = options.filter.filter(|filter| !filter.is_empty());
        options.order = options.order.filter(|order| !order.is_empty());
        let query = self.table.list(options);
        let client = self.client.ok_or(DbError::NoDbConnected)?;
        let rows = client
            .query(query.sql, &query.parameters)
            .await
            .map_err(backend)?;
        self.decode(&rows)
    }

    pub async fn find_many_in<V>(
        &self,
        column: KeyColumn<T::Column>,
        values: Vec<V>,
    ) -> Result<Vec<T::Model>, DbError<T::Error>>
    where
        V: Eq + Hash + Clone + ToSql + Sync + Send,
        Vec<V>: ToSql,
    {
        let client = self.client.ok_or(DbError::NoDbConnected)?;
        let mut seen = HashSet::new();
        let unique: Vec<V> = values
            .into_iter()
            .filter(|value| seen.insert(value.clone()))
            .collect();
        if unique.is_empty() {
            return Ok(Vec::new());
        }
        let rows = client
            .query(self.table.membership(column), &[&unique])
            .await
            .map_err(backend)?;
        self.decode(&rows)
    }

    fn decode(&self, rows: &[Row]) -> Result<Vec<T::Model>, DbError<T::Error>> {
        let models = rows
            .iter()
            .map(|row| self.table.decode(row))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| DbError::InvalidRecord { source })?;
        Ok(models.into_iter().flatten().collect())
    }
}
