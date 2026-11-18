use chrono::{DateTime, Utc};
use tokio_postgres::types::ToSql;

use crate::database_replica::client::DatabaseClient;

use crate::table_ops::access::{TEAM_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::RowDecoder;
use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};
use crate::team_rows::team::LiteLLMTeamTable;

const UNIQUE_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT t.*, to_jsonb(m.*) AS litellm_model_table FROM \"LiteLLM_TeamTable\" t LEFT JOIN \"LiteLLM_ModelTable\" m ON t.\"model_id\" = m.\"id\" WHERE t.\"team_id\" = $1) x";
const DUE_FOR_RESET_SQL: &str = "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t WHERE t.\"budget_duration\" IS NOT NULL AND (t.\"budget_reset_at\" IS NULL OR t.\"budget_reset_at\" < $1) LIMIT $2";
const FOR_USER_SQL: &str =
    "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t WHERE $1 = ANY(t.\"members\")";
const TEAMS_BY_IDS_SQL: &str = "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t WHERE t.\"team_id\" = ANY($1::text[])";
const ALL_TEAMS_SQL: &str = "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t LIMIT $1";

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn team_unique(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let found = self
            .find_team_rows("find_unique", UNIQUE_SQL, &[&team_id])
            .await?;
        Ok(found.into_iter().next())
    }

    pub async fn teams_due_for_reset(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let reset_at = reset_at.naive_utc();
        self.find_team_rows("find_many", DUE_FOR_RESET_SQL, &[&reset_at, &limit])
            .await
    }

    pub async fn teams_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.find_team_rows("find_many", FOR_USER_SQL, &[&user_id])
            .await
    }

    pub async fn teams_by_ids(
        &self,
        team_ids: &[String],
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.find_team_rows("find_many", TEAMS_BY_IDS_SQL, &[&team_ids])
            .await
    }

    pub async fn teams_all(&self) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let limit = self.settings.team_list_limit;
        self.find_team_rows("find_many", ALL_TEAMS_SQL, &[&limit])
            .await
    }

    async fn find_team_rows(
        &self,
        method: &str,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.query_rows(
            TEAM_ATTRIBUTE,
            method,
            sql,
            parameters,
            DATA_RETRY,
            OperationCallType::GetData,
            LiteLLMTeamTable::decode_row,
        )
        .await
    }
}
