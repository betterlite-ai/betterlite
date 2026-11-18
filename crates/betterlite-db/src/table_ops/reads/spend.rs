use crate::database_replica::client::DatabaseClient;

use crate::table_ops::access::{SPEND_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::{RowError, SpendLogRow, decode_model_rows};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const UNIQUE_SQL: &str =
    "SELECT row_to_json(s)::text FROM \"LiteLLM_SpendLogs\" s WHERE s.\"{column}\" = $1 LIMIT 1";
const MATCHING_SQL: &str =
    "SELECT row_to_json(s)::text FROM \"LiteLLM_SpendLogs\" s WHERE s.\"{column}\" = $1";
const ALL_SQL: &str =
    "SELECT row_to_json(s)::text FROM \"LiteLLM_SpendLogs\" s ORDER BY s.\"startTime\" DESC";

#[derive(Debug, Clone, Copy)]
pub enum SpendFilterColumn {
    RequestId,
    User,
    ApiKey,
    TeamId,
    Model,
    CallType,
    EndUser,
}

impl SpendFilterColumn {
    const fn name(self) -> &'static str {
        match self {
            Self::RequestId => "request_id",
            Self::User => "user",
            Self::ApiKey => "api_key",
            Self::TeamId => "team_id",
            Self::Model => "model",
            Self::CallType => "call_type",
            Self::EndUser => "end_user",
        }
    }
}

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn spend_unique(
        &self,
        column: SpendFilterColumn,
        value: &str,
    ) -> Result<Option<SpendLogRow>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.spend_unique_once(column, value)
        })
        .await
    }

    pub async fn spend_matching(
        &self,
        column: SpendFilterColumn,
        value: &str,
    ) -> Result<Vec<SpendLogRow>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.spend_matching_once(column, value)
        })
        .await
    }

    pub async fn spend_all(&self) -> Result<Vec<SpendLogRow>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.spend_all_once()
        })
        .await
    }

    async fn spend_unique_once(
        &self,
        column: SpendFilterColumn,
        value: &str,
    ) -> Result<Option<SpendLogRow>, TableOpsError<Client::Error>> {
        let sql = UNIQUE_SQL.replace("{column}", column.name());
        let found = self
            .text_row(SPEND_ATTRIBUTE, "find_unique", &sql, &[&value])
            .await?;
        match found {
            Some(text) => Ok(Some(serde_json::from_str(&text).map_err(RowError::Model)?)),
            None => Ok(None),
        }
    }

    async fn spend_matching_once(
        &self,
        column: SpendFilterColumn,
        value: &str,
    ) -> Result<Vec<SpendLogRow>, TableOpsError<Client::Error>> {
        let sql = MATCHING_SQL.replace("{column}", column.name());
        let rows = self
            .text_rows(SPEND_ATTRIBUTE, "find_many", &sql, &[&value])
            .await?;
        decode_model_rows(&rows)
    }

    async fn spend_all_once(&self) -> Result<Vec<SpendLogRow>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(SPEND_ATTRIBUTE, "find_many", ALL_SQL, &[])
            .await?;
        decode_model_rows(&rows)
    }
}
