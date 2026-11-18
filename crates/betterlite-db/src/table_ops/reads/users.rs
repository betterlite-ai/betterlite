use chrono::{DateTime, Utc};
use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::user::LiteLLMUserTable;

use crate::table_ops::access::{RAW_QUERY_ATTRIBUTE, TablePoolAccess, USER_ATTRIBUTE};
use crate::table_ops::rows::{RowDecoder, UserWithAliases};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const UNIQUE_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT u.*, COALESCE((SELECT json_agg(m.*) FROM \"LiteLLM_OrganizationMembership\" m WHERE m.\"user_id\" = u.\"user_id\"), '[]'::json) AS organization_memberships FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = $1) x";
const DUE_FOR_RESET_SQL: &str = "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"budget_duration\" IS NOT NULL AND (u.\"budget_reset_at\" IS NULL OR u.\"budget_reset_at\" < $1) LIMIT $2";
const USERS_BY_IDS_SQL: &str = "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = ANY($1::text[])";
const ALL_SQL: &str = "SELECT u.*, json_agg(v.key_alias) AS key_aliases FROM \"LiteLLM_UserTable\" u LEFT JOIN \"LiteLLM_VerificationToken\" v ON u.user_id = v.user_id GROUP BY u.user_id ORDER BY u.spend DESC LIMIT $1 OFFSET $2";

#[derive(Debug, Clone, Copy)]
pub enum UserFilter<'value> {
    UserId(&'value str),
    UserEmail(&'value str),
    SsoUserId(&'value str),
}

impl UserFilter<'_> {
    const fn column(self) -> (&'static str, &'static str) {
        match self {
            Self::UserId(value) => ("user_id", value),
            Self::UserEmail(value) => ("user_email", value),
            Self::SsoUserId(value) => ("sso_user_id", value),
        }
    }
}

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn user_unique(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let rows = self
            .find_users("find_unique", UNIQUE_SQL, &[&user_id])
            .await?;
        Ok(rows.into_iter().next())
    }

    pub async fn users_matching(
        &self,
        filter: UserFilter<'_>,
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let (column, value) = filter.column();
        let sql = format!(
            "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"{column}\" = $1"
        );
        self.find_users("find_many", &sql, &[&value]).await
    }

    pub async fn users_due_for_reset(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let reset_at = reset_at.naive_utc();
        self.find_users("find_many", DUE_FOR_RESET_SQL, &[&reset_at, &limit])
            .await
    }

    pub async fn users_by_ids(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        self.find_users("find_many", USERS_BY_IDS_SQL, &[&user_ids])
            .await
    }

    pub async fn users_all(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<UserWithAliases>, TableOpsError<Client::Error>> {
        self.query_rows(
            RAW_QUERY_ATTRIBUTE,
            "query_raw",
            ALL_SQL,
            &[&limit, &offset],
            DATA_RETRY,
            OperationCallType::GetData,
            UserWithAliases::decode_row,
        )
        .await
    }

    async fn find_users(
        &self,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        self.query_rows(
            USER_ATTRIBUTE,
            method,
            sql,
            parameters,
            DATA_RETRY,
            OperationCallType::GetData,
            LiteLLMUserTable::decode_row,
        )
        .await
    }
}
