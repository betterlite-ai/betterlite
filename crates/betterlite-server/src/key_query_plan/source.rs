use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::atomic::Ordering;

use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::repository::Repository;
use betterlite_db::postgres_store::rows::KeyColumn;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::user_repository::tables::USER_TABLE;

use crate::key_query_plan::error::KeyQueryPlanError;
use crate::key_query_plan::plan::{KeyQueryPlan, KeyTable};
use crate::key_query_plan::response::KeyRow;
use crate::key_query_plan::sql::{RenderedQuery, SqlValue, count_query, page_query};
use crate::object_permissions::attach::attach_object_permission_to_dict;

// the seam a key list reads through; production goes to Postgres and tests record the call order
pub trait KeyListSource {
    fn fetch_page(
        &self,
        plan: &KeyQueryPlan,
    ) -> impl Future<Output = Result<Vec<KeyRow>, KeyQueryPlanError>> + Send;

    fn count(
        &self,
        plan: &KeyQueryPlan,
    ) -> impl Future<Output = Result<i64, KeyQueryPlanError>> + Send;

    fn find_users(
        &self,
        ids: Vec<String>,
    ) -> impl Future<Output = Result<Vec<LiteLLMUserTable>, KeyQueryPlanError>> + Send;

    fn attach_object_permission(
        &self,
        row: &mut KeyRow,
    ) -> impl Future<Output = Result<(), KeyQueryPlanError>> + Send;
}

#[derive(Debug, Clone, Copy)]
pub struct PostgresKeyListSource<'db> {
    client: PrismaClient<'db>,
}

impl<'db> PostgresKeyListSource<'db> {
    #[must_use]
    pub const fn new(client: PrismaClient<'db>) -> Self {
        Self { client }
    }
}

impl KeyListSource for PostgresKeyListSource<'_> {
    fn fetch_page(
        &self,
        plan: &KeyQueryPlan,
    ) -> impl Future<Output = Result<Vec<KeyRow>, KeyQueryPlanError>> + Send {
        async move { fetch_page(self.client, plan).await }
    }

    fn count(
        &self,
        plan: &KeyQueryPlan,
    ) -> impl Future<Output = Result<i64, KeyQueryPlanError>> + Send {
        async move { count(self.client, plan).await }
    }

    fn find_users(
        &self,
        ids: Vec<String>,
    ) -> impl Future<Output = Result<Vec<LiteLLMUserTable>, KeyQueryPlanError>> + Send {
        async move { find_users(self.client, ids).await }
    }

    fn attach_object_permission(
        &self,
        row: &mut KeyRow,
    ) -> impl Future<Output = Result<(), KeyQueryPlanError>> + Send {
        async move { attach_permission(self.client, row).await }
    }
}

async fn fetch_page(
    client: PrismaClient<'_>,
    plan: &KeyQueryPlan,
) -> Result<Vec<KeyRow>, KeyQueryPlanError> {
    let query = page_query(plan);
    let session = read_store(client).session().await?;
    let rows = session
        .query(query.sql.as_str(), &bindings(&query))
        .await
        .map_err(backend::<Infallible>)?;
    match plan.table {
        KeyTable::Live => rows.iter().map(decode_live_row).collect(),
        KeyTable::Deleted => rows.iter().map(decode_archived_row).collect(),
    }
}

async fn count(client: PrismaClient<'_>, plan: &KeyQueryPlan) -> Result<i64, KeyQueryPlanError> {
    let query = count_query(plan);
    let session = read_store(client).session().await?;
    let row = session
        .query_one(query.sql.as_str(), &bindings(&query))
        .await
        .map_err(backend::<Infallible>)?;
    let total: i64 = row.try_get(0).map_err(backend::<Infallible>)?;
    Ok(total)
}

async fn find_users(
    client: PrismaClient<'_>,
    ids: Vec<String>,
) -> Result<Vec<LiteLLMUserTable>, KeyQueryPlanError> {
    let session = read_store(client).session().await?;
    let repository = Repository {
        client: Some(&session),
        table: USER_TABLE,
    };
    Ok(repository.find_many_in(KeyColumn::Id, ids).await?)
}

async fn attach_permission(
    client: PrismaClient<'_>,
    row: &mut KeyRow,
) -> Result<(), KeyQueryPlanError> {
    attach_object_permission_to_dict(row, Some(client)).await?;
    Ok(())
}

fn read_store<'db>(client: PrismaClient<'db>) -> &'db PostgresStore {
    if client
        .availability
        .reader_unavailable
        .load(Ordering::SeqCst)
    {
        client.writer
    } else {
        client.reader
    }
}

fn bindings(query: &RenderedQuery) -> Vec<&(dyn ToSql + Sync)> {
    query.params.iter().map(SqlValue::bind).collect()
}

fn decode_live_row(row: &Row) -> Result<KeyRow, KeyQueryPlanError> {
    let document: String = row.try_get(0).map_err(backend::<Infallible>)?;
    let mut fields = decode_mapping(&document)?;
    let budget: Option<String> = row.try_get(1).map_err(backend::<Infallible>)?;
    if let Some(document) = budget {
        let value: JsonData = serde_json::from_str(&document).map_err(KeyQueryPlanError::Row)?;
        fields.insert(String::from("litellm_budget_table"), value);
    }
    Ok(fields)
}

fn decode_archived_row(row: &Row) -> Result<KeyRow, KeyQueryPlanError> {
    let document: String = row.try_get(0).map_err(backend::<Infallible>)?;
    decode_mapping(&document)
}

fn decode_mapping(document: &str) -> Result<BTreeMap<String, JsonData>, KeyQueryPlanError> {
    let value: JsonData = serde_json::from_str(document).map_err(KeyQueryPlanError::Row)?;
    match value {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(KeyQueryPlanError::RowObject),
    }
}
