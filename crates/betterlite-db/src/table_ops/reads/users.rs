use chrono::{DateTime, Utc};

use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::user::LiteLLMUserTable;

use crate::table_ops::access::{RAW_QUERY_ATTRIBUTE, TablePoolAccess, USER_ATTRIBUTE};
use crate::table_ops::rows::{UserWithAliases, decode_user_row, decode_user_with_aliases};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const UNIQUE_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT u.*, COALESCE((SELECT json_agg(m.*) FROM \"LiteLLM_OrganizationMembership\" m WHERE m.\"user_id\" = u.\"user_id\"), '[]'::json) AS organization_memberships FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = $1) x";
const DUE_FOR_RESET_SQL: &str = "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"budget_duration\" IS NOT NULL AND (u.\"budget_reset_at\" IS NULL OR u.\"budget_reset_at\" < $1) LIMIT $2";
const IN_SQL: &str = "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = ANY($1::text[])";
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
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.user_unique_once(user_id)
        })
        .await
    }

    pub async fn users_matching(
        &self,
        filter: UserFilter<'_>,
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.users_matching_once(filter)
        })
        .await
    }

    pub async fn users_due_for_reset(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.users_due_for_reset_once(reset_at, limit)
        })
        .await
    }

    pub async fn users_by_ids(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.users_by_ids_once(user_ids)
        })
        .await
    }

    pub async fn users_all(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<UserWithAliases>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.users_all_once(limit, offset)
        })
        .await
    }

    async fn user_unique_once(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let found = self
            .text_row(USER_ATTRIBUTE, "find_unique", UNIQUE_SQL, &[&user_id])
            .await?;
        found
            .map(|text| decode_user_row(&text))
            .transpose()
            .map_err(Into::into)
    }

    async fn users_matching_once(
        &self,
        filter: UserFilter<'_>,
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let (column, value) = filter.column();
        let sql = format!(
            "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"{column}\" = $1"
        );
        let rows = self
            .text_rows(USER_ATTRIBUTE, "find_many", &sql, &[&value])
            .await?;
        rows.iter()
            .map(|text| decode_user_row(text).map_err(Into::into))
            .collect()
    }

    async fn users_due_for_reset_once(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let reset_at = reset_at.naive_utc();
        let rows = self
            .text_rows(
                USER_ATTRIBUTE,
                "find_many",
                DUE_FOR_RESET_SQL,
                &[&reset_at, &limit],
            )
            .await?;
        rows.iter()
            .map(|text| decode_user_row(text).map_err(Into::into))
            .collect()
    }

    async fn users_by_ids_once(
        &self,
        user_ids: &[String],
    ) -> Result<Vec<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(USER_ATTRIBUTE, "find_many", IN_SQL, &[&user_ids])
            .await?;
        rows.iter()
            .map(|text| decode_user_row(text).map_err(Into::into))
            .collect()
    }

    async fn users_all_once(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<UserWithAliases>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(
                RAW_QUERY_ATTRIBUTE,
                "query_raw",
                ALL_SQL,
                &[&limit, &offset],
            )
            .await?;
        rows.iter()
            .map(|text| decode_user_with_aliases(text).map_err(Into::into))
            .collect()
    }
}
