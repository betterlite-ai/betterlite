use crate::table_ops::access::TableAccess as _;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;
use crate::key_user_writes::hash::hash_token_if_needed;

use super::access::{KEY_ATTRIBUTE, TEAM_ATTRIBUTE, TablePoolAccess};
use super::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const DELETE_TOKENS_SQL: &str = "DELETE FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = ANY($1::text[]) RETURNING \"token\"";
const DELETE_OWNED_TOKENS_SQL: &str = "DELETE FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = ANY($1::text[]) AND \"user_id\" = $2 RETURNING \"token\"";
const DELETE_TEAM_KEYS_SQL: &str = "DELETE FROM \"LiteLLM_VerificationToken\" WHERE \"team_id\" = ANY($1::text[]) RETURNING \"token\"";
const DELETE_TEAMS_SQL: &str =
    "DELETE FROM \"LiteLLM_TeamTable\" WHERE \"team_id\" = ANY($1::text[]) RETURNING \"team_id\"";

pub trait TableDeletes<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn delete_keys(
        &self,
        tokens: &[SecretString],
        user_id: Option<&str>,
    ) -> impl Future<Output = Result<u64, TableOpsError<Client::Error>>> + Send;
    fn delete_keys_for_teams(
        &self,
        team_ids: &[String],
    ) -> impl Future<Output = Result<u64, TableOpsError<Client::Error>>> + Send;
    fn delete_teams(
        &self,
        team_ids: &[String],
    ) -> impl Future<Output = Result<Vec<String>, TableOpsError<Client::Error>>> + Send;
    fn delete_tokens(
        &self,
        sql: &str,
        attribute: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> impl Future<Output = Result<u64, TableOpsError<Client::Error>>> + Send;
}

impl<Client> TableDeletes<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn delete_keys(
        &self,
        tokens: &[SecretString],
        user_id: Option<&str>,
    ) -> Result<u64, TableOpsError<Client::Error>> {
        let hashed: Vec<String> = tokens
            .iter()
            .map(|token| hash_token_if_needed(token.expose_secret()))
            .collect();
        match user_id {
            Some(selected_user) => {
                self.delete_tokens(
                    DELETE_OWNED_TOKENS_SQL,
                    KEY_ATTRIBUTE,
                    &[&hashed, &selected_user],
                )
                .await
            }
            None => {
                self.delete_tokens(DELETE_TOKENS_SQL, KEY_ATTRIBUTE, &[&hashed])
                    .await
            }
        }
    }

    async fn delete_keys_for_teams(
        &self,
        team_ids: &[String],
    ) -> Result<u64, TableOpsError<Client::Error>> {
        self.delete_tokens(DELETE_TEAM_KEYS_SQL, KEY_ATTRIBUTE, &[&team_ids])
            .await
    }

    async fn delete_teams(
        &self,
        team_ids: &[String],
    ) -> Result<Vec<String>, TableOpsError<Client::Error>> {
        self.delete_tokens(DELETE_TEAMS_SQL, TEAM_ATTRIBUTE, &[&team_ids])
            .await?;
        Ok(team_ids.to_vec())
    }

    // every delete returns the removed keys, so one statement both filters and counts
    async fn delete_tokens(
        &self,
        sql: &str,
        attribute: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::DeleteData, || async {
            let deleted = self
                .text_rows(attribute, "delete_many", sql, parameters)
                .await?;
            Ok(u64::try_from(deleted.len()).unwrap_or(u64::MAX))
        })
        .await
    }
}
