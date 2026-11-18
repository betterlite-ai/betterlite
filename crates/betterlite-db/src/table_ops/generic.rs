use secrecy::{ExposeSecret as _, SecretString};

use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::decode_token_record;
use crate::resource_rows::config::ConfigRow;

use super::access::{
    CONFIG_ATTRIBUTE, KEY_ATTRIBUTE, SPEND_ATTRIBUTE, TablePoolAccess, USER_ATTRIBUTE,
};
use super::rows::{RowError, SpendLogRow, decode_user_row};
use super::{GENERIC_RETRY, OperationCallType, TableOps, TableOpsError};

const USER_FIRST_SQL: &str =
    "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = $1 LIMIT 1";
const KEY_FIRST_SQL: &str = "SELECT row_to_json(v)::text FROM \"LiteLLM_VerificationToken\" v WHERE v.\"token\" = $1 LIMIT 1";
const CONFIG_FIRST_SQL: &str =
    "SELECT row_to_json(c)::text FROM \"LiteLLM_Config\" c WHERE c.\"param_name\" = $1 LIMIT 1";
const SPEND_FIRST_SQL: &str =
    "SELECT row_to_json(s)::text FROM \"LiteLLM_SpendLogs\" s WHERE s.\"request_id\" = $1 LIMIT 1";

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn generic_user(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        self.attempt_with(GENERIC_RETRY, OperationCallType::GetGenericData, || {
            self.generic_user_once(user_id)
        })
        .await
    }

    pub async fn generic_key(
        &self,
        token: &SecretString,
    ) -> Result<Option<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.attempt_with(GENERIC_RETRY, OperationCallType::GetGenericData, || {
            self.generic_key_once(token.expose_secret())
        })
        .await
    }

    pub async fn generic_config(
        &self,
        param_name: &str,
    ) -> Result<Option<ConfigRow>, TableOpsError<Client::Error>> {
        self.attempt_with(GENERIC_RETRY, OperationCallType::GetGenericData, || {
            self.generic_config_once(param_name)
        })
        .await
    }

    pub async fn generic_spend(
        &self,
        request_id: &str,
    ) -> Result<Option<SpendLogRow>, TableOpsError<Client::Error>> {
        self.attempt_with(GENERIC_RETRY, OperationCallType::GetGenericData, || {
            self.generic_spend_once(request_id)
        })
        .await
    }

    async fn generic_user_once(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        let found = self
            .text_row(USER_ATTRIBUTE, "find_first", USER_FIRST_SQL, &[&user_id])
            .await?;
        match found {
            Some(text) => Ok(Some(decode_user_row(&text)?)),
            None => Ok(None),
        }
    }

    async fn generic_key_once(
        &self,
        stored: &str,
    ) -> Result<Option<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        let found = self
            .text_row(KEY_ATTRIBUTE, "find_first", KEY_FIRST_SQL, &[&stored])
            .await?;
        match found {
            Some(text) => Ok(Some(
                decode_token_record(text.as_bytes()).map_err(RowError::Key)?,
            )),
            None => Ok(None),
        }
    }

    async fn generic_config_once(
        &self,
        param_name: &str,
    ) -> Result<Option<ConfigRow>, TableOpsError<Client::Error>> {
        let found = self
            .text_row(
                CONFIG_ATTRIBUTE,
                "find_first",
                CONFIG_FIRST_SQL,
                &[&param_name],
            )
            .await?;
        match found {
            Some(text) => Ok(Some(
                serde_json::from_str::<ConfigRow>(&text).map_err(RowError::Model)?,
            )),
            None => Ok(None),
        }
    }

    async fn generic_spend_once(
        &self,
        request_id: &str,
    ) -> Result<Option<SpendLogRow>, TableOpsError<Client::Error>> {
        let found = self
            .text_row(
                SPEND_ATTRIBUTE,
                "find_first",
                SPEND_FIRST_SQL,
                &[&request_id],
            )
            .await?;
        match found {
            Some(text) => Ok(Some(
                serde_json::from_str::<SpendLogRow>(&text).map_err(RowError::Model)?,
            )),
            None => Ok(None),
        }
    }
}
