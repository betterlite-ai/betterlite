use crate::table_ops::access::ReadQuery;
use crate::table_ops::access::TableAccess as _;
use chrono::{DateTime, Utc};
use std::future::Future;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::LiteLLMBudgetTableFull;
use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::end_user::LiteLLMEndUserTable;

use crate::table_ops::access::{BUDGET_ATTRIBUTE, END_USER_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::RowDecoder;
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const BUDGETS_DUE_SQL: &str = "SELECT row_to_json(b)::text FROM \"LiteLLM_BudgetTable\" b WHERE b.\"budget_duration\" IS NOT NULL AND (b.\"budget_reset_at\" IS NULL OR b.\"budget_reset_at\" < $1) LIMIT $2";
const END_USERS_IN_SQL: &str = "SELECT row_to_json(e)::text FROM \"LiteLLM_EndUserTable\" e WHERE e.\"budget_id\" = ANY($1::text[])";

pub trait ResetReads<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn budgets_due_for_reset(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<Vec<LiteLLMBudgetTableFull>, TableOpsError<Client::Error>>> + Send;
    fn end_users_by_budgets(
        &self,
        budget_ids: &[String],
    ) -> impl Future<Output = Result<Vec<LiteLLMEndUserTable>, TableOpsError<Client::Error>>> + Send;
    fn find_models<Row>(
        &self,
        attribute: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<Vec<Row>, TableOpsError<Client::Error>>> + Send
    where
        Row: RowDecoder + Send;
}

impl<Client> ResetReads<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn budgets_due_for_reset(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMBudgetTableFull>, TableOpsError<Client::Error>> {
        let reset_moment = reset_at.naive_utc();
        self.find_models(BUDGET_ATTRIBUTE, BUDGETS_DUE_SQL, &[&reset_moment, &limit])
            .await
    }

    async fn end_users_by_budgets(
        &self,
        budget_ids: &[String],
    ) -> Result<Vec<LiteLLMEndUserTable>, TableOpsError<Client::Error>> {
        self.find_models(END_USER_ATTRIBUTE, END_USERS_IN_SQL, &[&budget_ids])
            .await
    }

    async fn find_models<Row>(
        &self,
        attribute: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, TableOpsError<Client::Error>>
    where
        Row: RowDecoder + Send,
    {
        self.query_rows(
            ReadQuery {
                attribute,
                method: "find_many",
                sql,
                parameters,
            },
            DATA_RETRY,
            OperationCallType::GetData,
            Row::decode_row,
        )
        .await
    }
}
