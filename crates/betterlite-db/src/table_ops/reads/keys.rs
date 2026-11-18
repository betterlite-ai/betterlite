use std::error::Error;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::key_repository::decode::decode_token_record;
use crate::key_user_writes::hash::hash_token_if_needed;

use crate::table_ops::access::{KEY_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::RowError;
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const BY_USER_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" WHERE v.\"user_id\" = $1) x";
const BY_TEAM_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" WHERE v.\"team_id\" = $1 LIMIT $2) x";
const DUE_FOR_RESET_SQL: &str = "SELECT row_to_json(v)::text FROM \"LiteLLM_VerificationToken\" v WHERE (v.\"expires\" IS NULL OR v.\"expires\" > $1) AND v.\"budget_reset_at\" < $2 AND v.\"budget_duration\" IS NOT NULL LIMIT $3";
const ALL_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" ORDER BY v.\"spend\" DESC) x";
const IN_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" WHERE v.\"token\" = ANY($1::text[]) ORDER BY v.\"spend\" DESC) x";

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn keys_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.keys_for_user_once(user_id)
        })
        .await
    }

    pub async fn keys_for_team(
        &self,
        team_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.keys_for_team_once(team_id, limit)
        })
        .await
    }

    pub async fn keys_due_for_reset(
        &self,
        expires: DateTime<Utc>,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.keys_due_for_reset_once(expires, reset_at, limit)
        })
        .await
    }

    pub async fn keys_matching(
        &self,
        tokens: Option<&[SecretString]>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.keys_matching_once(tokens)
        })
        .await
    }

    async fn keys_for_user_once(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(KEY_ATTRIBUTE, "find_many", BY_USER_SQL, &[&user_id])
            .await?;
        decode_tokens(&rows)
    }

    async fn keys_for_team_once(
        &self,
        team_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(KEY_ATTRIBUTE, "find_many", BY_TEAM_SQL, &[&team_id, &limit])
            .await?;
        decode_tokens(&rows)
    }

    async fn keys_due_for_reset_once(
        &self,
        expires: DateTime<Utc>,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        let expires_at = expires.naive_utc();
        let reset_at = reset_at.naive_utc();
        let rows = self
            .text_rows(
                KEY_ATTRIBUTE,
                "find_many",
                DUE_FOR_RESET_SQL,
                &[&expires_at, &reset_at, &limit],
            )
            .await?;
        decode_tokens(&rows)
    }

    async fn keys_matching_once(
        &self,
        tokens: Option<&[SecretString]>,
    ) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        let rows = match tokens {
            None => {
                self.text_rows(KEY_ATTRIBUTE, "find_many", ALL_SQL, &[])
                    .await?
            }
            Some(tokens) => {
                let hashed: Vec<String> = tokens
                    .iter()
                    .map(|token| hash_token_if_needed(token.expose_secret()))
                    .collect();
                self.text_rows(KEY_ATTRIBUTE, "find_many", IN_SQL, &[&hashed])
                    .await?
            }
        };
        decode_tokens(&rows)
    }
}

fn decode_tokens<ClientError>(
    rows: &[String],
) -> Result<Vec<LiteLLMVerificationToken>, TableOpsError<ClientError>>
where
    ClientError: Error + Send + Sync + 'static,
{
    rows.iter()
        .map(|text| {
            decode_token_record(text.as_bytes())
                .map_err(RowError::Key)
                .map_err(Into::into)
        })
        .collect()
}
