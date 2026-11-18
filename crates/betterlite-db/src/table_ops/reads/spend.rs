use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;

use crate::table_ops::access::{SPEND_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::{RowDecoder, SpendLogRow};
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};

const UNIQUE_SQL: &str =
    "SELECT row_to_json(s)::text FROM \"LiteLLM_SpendLogs\" s WHERE s.\"{column}\" = $1 LIMIT 1";
const MATCHING_SQL: &str =
    "SELECT row_to_json(s)::text FROM \"LiteLLM_SpendLogs\" s WHERE s.\"{column}\" = $1";
const ALL_SPEND_LOGS_SQL: &str =
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

// LiteLLM's get_data picks find_unique or find_all for the spend table through these two queries
#[derive(Debug, Clone, Copy)]
pub struct SpendQuery {
    template: &'static str,
    method: &'static str,
}

impl SpendQuery {
    pub const UNIQUE: Self = Self {
        template: UNIQUE_SQL,
        method: "find_unique",
    };
    pub const MATCHING: Self = Self {
        template: MATCHING_SQL,
        method: "find_many",
    };
}

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn spend_rows(
        &self,
        query: SpendQuery,
        column: SpendFilterColumn,
        value: &str,
    ) -> Result<Vec<SpendLogRow>, TableOpsError<Client::Error>> {
        self.find_spend_logs(
            query.method,
            &query.template.replace("{column}", column.name()),
            &[&value],
        )
        .await
    }

    pub async fn spend_all(&self) -> Result<Vec<SpendLogRow>, TableOpsError<Client::Error>> {
        self.find_spend_logs("find_many", ALL_SPEND_LOGS_SQL, &[])
            .await
    }

    async fn find_spend_logs(
        &self,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<SpendLogRow>, TableOpsError<Client::Error>> {
        self.query_rows(
            SPEND_ATTRIBUTE,
            method,
            sql,
            parameters,
            DATA_RETRY,
            OperationCallType::GetData,
            SpendLogRow::decode_row,
        )
        .await
    }
}
