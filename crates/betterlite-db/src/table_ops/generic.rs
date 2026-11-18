use secrecy::{ExposeSecret as _, SecretString};

use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::resource_rows::config::ConfigRow;

use super::access::{
    CONFIG_ATTRIBUTE, KEY_ATTRIBUTE, SPEND_ATTRIBUTE, TablePoolAccess, USER_ATTRIBUTE,
};
use super::rows::{RowDecoder, RowError, SpendLogRow};
use super::{GENERIC_RETRY, OperationCallType, TableOps, TableOpsError};

const USER_FIRST_SQL: &str =
    "SELECT row_to_json(u)::text FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = $1 LIMIT 1";
const KEY_FIRST_SQL: &str = "SELECT row_to_json(v)::text FROM \"LiteLLM_VerificationToken\" v WHERE v.\"token\" = $1 LIMIT 1";
const CONFIG_FIRST_SQL: &str =
    "SELECT row_to_json(c)::text FROM \"LiteLLM_Config\" c WHERE c.\"param_name\" = $1 LIMIT 1";
const SPEND_FIRST_SQL: &str =
    "SELECT row_to_json(s)::text FROM \"LiteLLM_SpendLogs\" s WHERE s.\"request_id\" = $1 LIMIT 1";

// each table's find_first runs on its own routed handle over its own table
struct GenericSource {
    attribute: &'static str,
    sql: &'static str,
}

const USER_SOURCE: GenericSource = GenericSource {
    attribute: USER_ATTRIBUTE,
    sql: USER_FIRST_SQL,
};
const KEY_SOURCE: GenericSource = GenericSource {
    attribute: KEY_ATTRIBUTE,
    sql: KEY_FIRST_SQL,
};
const CONFIG_SOURCE: GenericSource = GenericSource {
    attribute: CONFIG_ATTRIBUTE,
    sql: CONFIG_FIRST_SQL,
};
const SPEND_SOURCE: GenericSource = GenericSource {
    attribute: SPEND_ATTRIBUTE,
    sql: SPEND_FIRST_SQL,
};

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn generic_user(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, TableOpsError<Client::Error>> {
        self.find_generic(USER_SOURCE, user_id, LiteLLMUserTable::decode_row)
            .await
    }

    pub async fn generic_key(
        &self,
        token: &SecretString,
    ) -> Result<Option<LiteLLMVerificationToken>, TableOpsError<Client::Error>> {
        self.find_generic(
            KEY_SOURCE,
            token.expose_secret(),
            LiteLLMVerificationToken::decode_row,
        )
        .await
    }

    pub async fn generic_config(
        &self,
        param_name: &str,
    ) -> Result<Option<ConfigRow>, TableOpsError<Client::Error>> {
        self.find_generic(CONFIG_SOURCE, param_name, ConfigRow::decode_row)
            .await
    }

    pub async fn generic_spend(
        &self,
        request_id: &str,
    ) -> Result<Option<SpendLogRow>, TableOpsError<Client::Error>> {
        self.find_generic(SPEND_SOURCE, request_id, SpendLogRow::decode_row)
            .await
    }

    async fn find_generic<Row, Parse>(
        &self,
        source: GenericSource,
        value: &str,
        parse: Parse,
    ) -> Result<Option<Row>, TableOpsError<Client::Error>>
    where
        Row: Send,
        Parse: Fn(&str) -> Result<Row, RowError> + Send + Sync,
    {
        let rows = self
            .query_rows(
                source.attribute,
                "find_first",
                source.sql,
                &[&value],
                GENERIC_RETRY,
                OperationCallType::GetGenericData,
                parse,
            )
            .await?;
        Ok(rows.into_iter().next())
    }
}
