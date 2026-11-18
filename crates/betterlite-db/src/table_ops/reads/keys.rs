use crate::table_ops::access::ReadQuery;
use crate::table_ops::access::TableAccess as _;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::key_user_writes::hash::hash_token_if_needed;

use crate::table_ops::access::{KEY_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::RowDecoder as _;
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const BY_USER_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" WHERE v.\"user_id\" = $1) x";
const BY_TEAM_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" WHERE v.\"team_id\" = $1 LIMIT $2) x";
const DUE_FOR_RESET_SQL: &str = "SELECT row_to_json(v)::text FROM \"LiteLLM_VerificationToken\" v WHERE (v.\"expires\" IS NULL OR v.\"expires\" > $1) AND v.\"budget_reset_at\" < $2 AND v.\"budget_duration\" IS NOT NULL LIMIT $3";
const ALL_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" ORDER BY v.\"spend\" DESC) x";
const IN_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" WHERE v.\"token\" = ANY($1::text[]) ORDER BY v.\"spend\" DESC) x";

pub trait KeyReads<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn keys_for_user(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>>> + Send;
    fn keys_for_team(
        &self,
        team_id: &str,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>>> + Send;
    fn keys_due_for_reset(
        &self,
        expires: DateTime<Utc>,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>>> + Send;
    fn keys_matching(
        &self,
        tokens: Option<&[SecretString]>,
    ) -> impl Future<Output = Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>>> + Send;
    fn find_tokens(
        &self,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>>> + Send;
}

impl<Client> KeyReads<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn keys_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.find_tokens("find_many", BY_USER_SQL, &[&user_id])
            .await
    }

    async fn keys_for_team(
        &self,
        team_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.find_tokens("find_many", BY_TEAM_SQL, &[&team_id, &limit])
            .await
    }

    async fn keys_due_for_reset(
        &self,
        expires: DateTime<Utc>,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        let expires_at = expires.naive_utc();
        let reset_moment = reset_at.naive_utc();
        self.find_tokens(
            "find_many",
            DUE_FOR_RESET_SQL,
            &[&expires_at, &reset_moment, &limit],
        )
        .await
    }

    async fn keys_matching(
        &self,
        tokens: Option<&[SecretString]>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        let Some(selected) = tokens else {
            return self.find_tokens("find_many", ALL_SQL, &[]).await;
        };
        let hashed: Vec<String> = selected
            .iter()
            .map(|token| hash_token_if_needed(token.expose_secret()))
            .collect();
        self.find_tokens("find_many", IN_SQL, &[&hashed]).await
    }

    async fn find_tokens(
        &self,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.query_rows(
            ReadQuery {
                attribute: KEY_ATTRIBUTE,
                method,
                sql,
                parameters,
            },
            DATA_RETRY,
            OperationCallType::GetData,
            LiteLLMVerificationToken::decode_row,
        )
        .await
    }
}
