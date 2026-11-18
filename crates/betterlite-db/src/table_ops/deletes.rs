use secrecy::{ExposeSecret as _, SecretString};

use crate::database_replica::client::DatabaseClient;
use crate::key_user_writes::hash::hash_token_if_needed;

use super::access::{KEY_ATTRIBUTE, TEAM_ATTRIBUTE, TablePoolAccess};
use super::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const DELETE_TOKENS_SQL: &str =
    "DELETE FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = ANY($1::text[])";
const DELETE_OWNED_TOKENS_SQL: &str = "DELETE FROM \"LiteLLM_VerificationToken\" WHERE \"token\" = ANY($1::text[]) AND \"user_id\" = $2";
const DELETE_TEAM_KEYS_SQL: &str =
    "DELETE FROM \"LiteLLM_VerificationToken\" WHERE \"team_id\" = ANY($1::text[])";
const DELETE_TEAMS_SQL: &str =
    "DELETE FROM \"LiteLLM_TeamTable\" WHERE \"team_id\" = ANY($1::text[])";

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn delete_keys(
        &self,
        tokens: &[SecretString],
        user_id: Option<&str>,
    ) -> Result<u64, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::DeleteData, || {
            self.delete_keys_once(tokens, user_id)
        })
        .await
    }

    pub async fn delete_keys_for_teams(
        &self,
        team_ids: &[String],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::DeleteData, || {
            self.delete_keys_for_teams_once(team_ids)
        })
        .await
    }

    pub async fn delete_teams(
        &self,
        team_ids: &[String],
    ) -> Result<Vec<String>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::DeleteData, || {
            self.delete_teams_once(team_ids)
        })
        .await
    }

    async fn delete_keys_once(
        &self,
        tokens: &[SecretString],
        user_id: Option<&str>,
    ) -> Result<u64, TableOpsError<Client::Error>> {
        let hashed: Vec<String> = tokens
            .iter()
            .map(|token| hash_token_if_needed(token.expose_secret()))
            .collect();
        match user_id {
            Some(user_id) => {
                self.execute(
                    KEY_ATTRIBUTE,
                    "delete_many",
                    DELETE_OWNED_TOKENS_SQL,
                    &[&hashed, &user_id],
                )
                .await
            }
            None => {
                self.execute(KEY_ATTRIBUTE, "delete_many", DELETE_TOKENS_SQL, &[&hashed])
                    .await
            }
        }
    }

    async fn delete_keys_for_teams_once(
        &self,
        team_ids: &[String],
    ) -> Result<(), TableOpsError<Client::Error>> {
        self.execute(
            KEY_ATTRIBUTE,
            "delete_many",
            DELETE_TEAM_KEYS_SQL,
            &[&team_ids],
        )
        .await?;
        Ok(())
    }

    async fn delete_teams_once(
        &self,
        team_ids: &[String],
    ) -> Result<Vec<String>, TableOpsError<Client::Error>> {
        self.execute(
            TEAM_ATTRIBUTE,
            "delete_many",
            DELETE_TEAMS_SQL,
            &[&team_ids],
        )
        .await?;
        Ok(team_ids.to_vec())
    }
}
