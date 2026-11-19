use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::managed_rows::VectorStoreRow;
use crate::auth_read::include::{Including, IncludingQuery};

const OBJECT_PERMISSION_ONE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ObjectPermissionTable\" AS entry WHERE entry.\"object_permission_id\" = $1";

const OBJECT_PERMISSION_LIST_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_ObjectPermissionTable\" AS entry LIMIT $1";

const OBJECT_PERMISSION_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_ObjectPermissionTable\" AS entry WHERE entry.\"object_permission_id\" = $1 RETURNING row_to_json(entry)::text";

const OBJECT_PERMISSION_UPDATE_SQL: &str = "UPDATE \"LiteLLM_ObjectPermissionTable\" AS entry SET \"object_permission_id\" = \"object_permission_id\" WHERE entry.\"object_permission_id\" = $1 RETURNING row_to_json(entry)::text";

const OBJECT_PERMISSION_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_ObjectPermissionTable\" AS entry DEFAULT VALUES RETURNING row_to_json(entry)::text";

const OBJECT_PERMISSION_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_ObjectPermissionTable\"";

const OBJECT_PERMISSION_ANY_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ObjectPermissionTable\" AS entry WHERE entry.\"object_permission_id\" = ANY($1::text[])";

const MANAGED_STORE_ONE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS entry WHERE entry.\"vector_store_id\" = $1";

const MANAGED_STORE_LIST_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS entry LIMIT $1";

const MANAGED_STORE_IN_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS entry WHERE entry.\"vector_store_id\" = ANY($1::text[]) LIMIT $2";

const MANAGED_STORE_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_ManagedVectorStoresTable\" AS entry WHERE entry.\"vector_store_id\" = $1 RETURNING row_to_json(entry)::text";

const MANAGED_STORE_UPDATE_SQL: &str = "UPDATE \"LiteLLM_ManagedVectorStoresTable\" AS entry SET \"vector_store_id\" = \"vector_store_id\" WHERE entry.\"vector_store_id\" = $1 RETURNING row_to_json(entry)::text";

const MANAGED_STORE_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_ManagedVectorStoresTable\" AS entry DEFAULT VALUES RETURNING row_to_json(entry)::text";

const MANAGED_STORE_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_ManagedVectorStoresTable\"";

const MANAGED_STORE_COUNT_IN_SQL: &str = "SELECT count(*) FROM \"LiteLLM_ManagedVectorStoresTable\" AS entry WHERE entry.\"vector_store_id\" = ANY($1::text[])";

const MANAGED_STORE_MEMBERSHIP_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ManagedVectorStoresTable\" AS entry WHERE entry.\"vector_store_id\" = ANY($1::text[])";

// the row_to_json column every read selects, or the reason it could not be read
#[derive(Debug, thiserror::Error)]
pub enum RowError {
    #[error("stored row column could not be read")]
    Column(#[source] DriverError),
    #[error("stored row is not a readable record")]
    Record(#[source] serde_json::Error),
}

fn row_text(row: &Row) -> Result<String, RowError> {
    row.try_get(0)
        .map_err(|driver| RowError::Column(DriverError { driver }))
}

#[derive(Clone, Copy)]
pub struct ObjectPermissionTable;

#[derive(Clone, Copy)]
pub struct ManagedVectorStoreTable;

impl RowCodec for ObjectPermissionTable {
    type Model = LiteLLMObjectPermissionTable;
    type Error = RowError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        let text = row_text(row)?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(RowError::Record)
    }
}

// a managed vector store row as Prisma hands it over: its model_dump projection and nothing else yet
impl RowCodec for ManagedVectorStoreTable {
    type Model = VectorStoreRow;
    type Error = RowError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        let record: JsonData = serde_json::from_str(&row_text(row)?).map_err(RowError::Record)?;
        Ok(Some(VectorStoreRow {
            model_dump: Some(record),
            dict: None,
            fields: BTreeMap::new(),
        }))
    }
}

const fn object_permission_keyed_sql(action: KeyAction<'_, ()>) -> &'static str {
    match action {
        KeyAction::Find => OBJECT_PERMISSION_ONE_SQL,
        KeyAction::Delete => OBJECT_PERMISSION_DELETE_SQL,
        KeyAction::Update(()) => OBJECT_PERMISSION_UPDATE_SQL,
    }
}

const fn managed_store_keyed_sql(action: KeyAction<'_, ()>) -> &'static str {
    match action {
        KeyAction::Find => MANAGED_STORE_ONE_SQL,
        KeyAction::Delete => MANAGED_STORE_DELETE_SQL,
        KeyAction::Update(()) => MANAGED_STORE_UPDATE_SQL,
    }
}

fn keyed_query<'query>(sql: &'query str, value: &'query (dyn ToSql + Sync)) -> Query<'query> {
    Query {
        sql,
        parameters: vec![value],
    }
}

impl Table for ObjectPermissionTable {
    type Column = Infallible;
    type Filter = Infallible;
    type Order = Infallible;
    type Data = ();

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, ()>,
        key: Key<'query, Self::Column>,
    ) -> Query<'query> {
        keyed_query(object_permission_keyed_sql(action), key.value)
    }

    fn list<'query>(
        &'query self,
        _options: FindMany<'query, Self::Filter, Self::Order>,
    ) -> Query<'query> {
        object_permission_listed(None, None)
    }

    fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
        Query {
            sql: OBJECT_PERMISSION_INSERT_SQL,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&self, _filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        Query {
            sql: OBJECT_PERMISSION_COUNT_SQL,
            parameters: Vec::new(),
        }
    }

    fn membership(&self, _column: KeyColumn<Self::Column>) -> &str {
        OBJECT_PERMISSION_ANY_SQL
    }
}

impl Table for ManagedVectorStoreTable {
    type Column = Infallible;
    type Filter = Vec<String>;
    type Order = Infallible;
    type Data = ();

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, ()>,
        key: Key<'query, Self::Column>,
    ) -> Query<'query> {
        keyed_query(managed_store_keyed_sql(action), key.value)
    }

    // a bare table read has no borrowed limit, so the selecting table applies the request's take
    fn list<'query>(
        &'query self,
        options: FindMany<'query, Self::Filter, Self::Order>,
    ) -> Query<'query> {
        let filter = options.filter.and_then(|filters| filters.first());
        managed_store_list(filter, None)
    }

    fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
        Query {
            sql: MANAGED_STORE_INSERT_SQL,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&self, filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        let Some(ids) = filter.and_then(|filters| filters.first()) else {
            return Query {
                sql: MANAGED_STORE_COUNT_SQL,
                parameters: Vec::new(),
            };
        };
        Query {
            sql: MANAGED_STORE_COUNT_IN_SQL,
            parameters: vec![ids],
        }
    }

    fn membership(&self, _column: KeyColumn<Self::Column>) -> &str {
        MANAGED_STORE_MEMBERSHIP_SQL
    }
}

// both grant tables answer the same relation selection, and only the listed read differs
macro_rules! grant_including {
    ($table:ty, $listed:path) => {
        impl Including for $table {
            type Include = ();

            fn including<'query>(
                &'query self,
                query: IncludingQuery<'query, Self>,
                _include: Option<&'query Self::Include>,
            ) -> Query<'query> {
                match query {
                    IncludingQuery::Keyed { action, key } => self.keyed(action, key),
                    IncludingQuery::Listed { filter, limit } => $listed(filter, limit.as_ref()),
                    IncludingQuery::Inserted { data } => self.insert(data),
                }
            }
        }
    };
}

grant_including!(ObjectPermissionTable, object_permission_listed);
grant_including!(ManagedVectorStoreTable, managed_store_list);

// Prisma's absent take means no limit, and Postgres returns the same rows for the largest row count
const NO_TAKE: i64 = i64::MAX;

// the grant macro passes both tables a filter; this one has none, so only the limit binds
fn object_permission_listed<'query>(
    _filter: Option<&'query Infallible>,
    limit: Option<&'query i64>,
) -> Query<'query> {
    Query {
        sql: OBJECT_PERMISSION_LIST_SQL,
        parameters: vec![limit.unwrap_or(&NO_TAKE)],
    }
}

#[must_use]
pub fn managed_store_list<'query>(
    filter: Option<&'query Vec<String>>,
    limit: Option<&'query i64>,
) -> Query<'query> {
    let take = limit.unwrap_or(&NO_TAKE);
    let (sql, parameters): (&'query str, Vec<&'query (dyn ToSql + Sync)>) = filter.map_or_else(
        || (MANAGED_STORE_LIST_SQL, vec![take]),
        |ids| (MANAGED_STORE_IN_SQL, vec![ids, take]),
    );
    Query { sql, parameters }
}
