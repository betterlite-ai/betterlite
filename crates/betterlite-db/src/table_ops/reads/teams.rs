use chrono::{DateTime, Utc};

use crate::database_replica::client::DatabaseClient;
use crate::table_ops::access::{TEAM_ATTRIBUTE, TablePoolAccess};
use crate::table_ops::rows::decode_team_row;

use crate::table_ops::{DATA_RETRY, OperationCallType, TableOps, TableOpsError};
use crate::team_rows::team::LiteLLMTeamTable;

const UNIQUE_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT t.*, to_jsonb(m.*) AS litellm_model_table FROM \"LiteLLM_TeamTable\" t LEFT JOIN \"LiteLLM_ModelTable\" m ON t.\"model_id\" = m.\"id\" WHERE t.\"team_id\" = $1) x";
const DUE_FOR_RESET_SQL: &str = "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t WHERE t.\"budget_duration\" IS NOT NULL AND (t.\"budget_reset_at\" IS NULL OR t.\"budget_reset_at\" < $1) LIMIT $2";
const FOR_USER_SQL: &str =
    "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t WHERE $1 = ANY(t.\"members\")";
const IN_SQL: &str = "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t WHERE t.\"team_id\" = ANY($1::text[])";
const ALL_SQL: &str = "SELECT row_to_json(t)::text FROM \"LiteLLM_TeamTable\" t LIMIT $1";

impl<Client> TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    pub async fn team_unique(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.team_unique_once(team_id)
        })
        .await
    }

    pub async fn teams_due_for_reset(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.teams_due_for_reset_once(reset_at, limit)
        })
        .await
    }

    pub async fn teams_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.teams_for_user_once(user_id)
        })
        .await
    }

    pub async fn teams_by_ids(
        &self,
        team_ids: &[String],
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.teams_by_ids_once(team_ids)
        })
        .await
    }

    pub async fn teams_all(&self) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        self.attempt_with(DATA_RETRY, OperationCallType::GetData, || {
            self.teams_all_once()
        })
        .await
    }

    async fn team_unique_once(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let found = self
            .text_row(TEAM_ATTRIBUTE, "find_unique", UNIQUE_SQL, &[&team_id])
            .await?;
        found
            .map(|text| decode_team_row(&text))
            .transpose()
            .map_err(Into::into)
    }

    async fn teams_due_for_reset_once(
        &self,
        reset_at: DateTime<Utc>,
        limit: Option<i64>,
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let reset_at = reset_at.naive_utc();
        let rows = self
            .text_rows(
                TEAM_ATTRIBUTE,
                "find_many",
                DUE_FOR_RESET_SQL,
                &[&reset_at, &limit],
            )
            .await?;
        decode_teams(&rows)
    }

    async fn teams_for_user_once(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(TEAM_ATTRIBUTE, "find_many", FOR_USER_SQL, &[&user_id])
            .await?;
        decode_teams(&rows)
    }

    async fn teams_by_ids_once(
        &self,
        team_ids: &[String],
    ) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let rows = self
            .text_rows(TEAM_ATTRIBUTE, "find_many", IN_SQL, &[&team_ids])
            .await?;
        decode_teams(&rows)
    }

    async fn teams_all_once(&self) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<Client::Error>> {
        let limit = self.settings.team_list_limit;
        let rows = self
            .text_rows(TEAM_ATTRIBUTE, "find_many", ALL_SQL, &[&limit])
            .await?;
        decode_teams(&rows)
    }
}

fn decode_teams<ClientError>(
    rows: &[String],
) -> Result<Vec<LiteLLMTeamTable>, TableOpsError<ClientError>>
where
    ClientError: std::error::Error + Send + Sync + 'static,
{
    rows.iter()
        .map(|text| decode_team_row(text).map_err(Into::into))
        .collect()
}
