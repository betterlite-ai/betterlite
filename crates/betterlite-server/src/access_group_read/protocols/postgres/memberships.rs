use deadpool_postgres::GenericClient;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::access_group_read::protocols::error::AccessGroupStoreError;
use crate::access_group_read::protocols::records::{AgentRecord, KeyRecord, TeamRecord};
use crate::access_group_read::protocols::tables::{AgentTable, KeyTable, TeamTable, TeamWhere};

use super::PgAccessGroupTables;

const FIND_TEAM_SQL: &str = r#"SELECT "team_id", "team_alias", "access_group_ids" FROM "LiteLLM_TeamTable" WHERE "team_id" = $1"#;
const FIND_TEAMS_BY_IDS_SQL: &str = r#"SELECT "team_id", "team_alias", "access_group_ids" FROM "LiteLLM_TeamTable" WHERE "team_id" = ANY($1::text[])"#;
const FIND_TEAMS_CARRYING_SQL: &str = r#"SELECT "team_id", "team_alias", "access_group_ids" FROM "LiteLLM_TeamTable" WHERE "access_group_ids" && $1::text[]"#;
const FIND_TEAMS_CARRYING_OR_IDS_SQL: &str = r#"SELECT "team_id", "team_alias", "access_group_ids" FROM "LiteLLM_TeamTable"
WHERE "access_group_ids" && $1::text[] OR "team_id" = ANY($2::text[])"#;
const SET_TEAM_GROUPS_SQL: &str = r#"UPDATE "LiteLLM_TeamTable" SET "access_group_ids" = $2, "updated_at" = (NOW() AT TIME ZONE 'UTC') WHERE "team_id" = $1"#;
const FIND_KEY_SQL: &str =
    r#"SELECT "token", "access_group_ids" FROM "LiteLLM_VerificationToken" WHERE "token" = $1"#;
const FIND_KEYS_CARRYING_SQL: &str = r#"SELECT "token", "access_group_ids" FROM "LiteLLM_VerificationToken" WHERE "access_group_ids" && $1::text[]"#;
const SET_KEY_GROUPS_SQL: &str = r#"UPDATE "LiteLLM_VerificationToken" SET "access_group_ids" = $2, "updated_at" = (NOW() AT TIME ZONE 'UTC') WHERE "token" = $1"#;
const FIND_AGENTS_CARRYING_SQL: &str = r#"SELECT "agent_id", "access_group_ids" FROM "LiteLLM_AgentsTable" WHERE "access_group_ids" && $1::text[]"#;
const SET_AGENT_GROUPS_SQL: &str = r#"UPDATE "LiteLLM_AgentsTable" SET "access_group_ids" = $2, "updated_at" = (NOW() AT TIME ZONE 'UTC') WHERE "agent_id" = $1"#;

#[derive(Clone, Copy)]
pub(super) enum Membership {
    Team,
    Key,
    Agent,
}

impl Membership {
    pub(super) const fn statement(self) -> &'static str {
        match self {
            Self::Team => SET_TEAM_GROUPS_SQL,
            Self::Key => SET_KEY_GROUPS_SQL,
            Self::Agent => SET_AGENT_GROUPS_SQL,
        }
    }
}

impl<C: GenericClient + Sync> TeamTable for PgAccessGroupTables<'_, C> {
    async fn find(&self, team_id: &str) -> Result<Option<TeamRecord>, AccessGroupStoreError> {
        let parameters: [&(dyn ToSql + Sync); 1] = [&team_id];
        let rows = self.query(FIND_TEAM_SQL, &parameters).await?;
        rows.first().map(TeamRecord::from_row).transpose()
    }

    async fn find_many(
        &self,
        filter: &TeamWhere,
    ) -> Result<Vec<TeamRecord>, AccessGroupStoreError> {
        let rows = match filter {
            TeamWhere::IdIn(ids) => {
                let parameters: [&(dyn ToSql + Sync); 1] = [ids];
                self.query(FIND_TEAMS_BY_IDS_SQL, &parameters).await?
            }
            TeamWhere::CarryingAny(groups) => {
                let parameters: [&(dyn ToSql + Sync); 1] = [groups];
                self.query(FIND_TEAMS_CARRYING_SQL, &parameters).await?
            }
            TeamWhere::CarryingAnyOrIdIn { groups, ids } => {
                let parameters: [&(dyn ToSql + Sync); 2] = [groups, ids];
                self.query(FIND_TEAMS_CARRYING_OR_IDS_SQL, &parameters)
                    .await?
            }
        };
        rows.iter().map(TeamRecord::from_row).collect()
    }

    async fn set_access_group_ids(
        &self,
        team_id: &str,
        ids: &[String],
    ) -> Result<(), AccessGroupStoreError> {
        self.write_list(Membership::Team, team_id, ids).await
    }
}

impl<C: GenericClient + Sync> KeyTable for PgAccessGroupTables<'_, C> {
    async fn find(&self, token: &str) -> Result<Option<KeyRecord>, AccessGroupStoreError> {
        let parameters: [&(dyn ToSql + Sync); 1] = [&token];
        let rows = self.query(FIND_KEY_SQL, &parameters).await?;
        rows.first().map(KeyRecord::from_row).transpose()
    }

    async fn find_carrying(
        &self,
        access_group_id: &str,
    ) -> Result<Vec<KeyRecord>, AccessGroupStoreError> {
        self.carrying_records(FIND_KEYS_CARRYING_SQL, access_group_id, KeyRecord::from_row)
            .await
    }

    async fn set_access_group_ids(
        &self,
        token: &str,
        ids: &[String],
    ) -> Result<(), AccessGroupStoreError> {
        self.write_list(Membership::Key, token, ids).await
    }
}

impl<C: GenericClient + Sync> AgentTable for PgAccessGroupTables<'_, C> {
    async fn find_carrying(
        &self,
        access_group_id: &str,
    ) -> Result<Vec<AgentRecord>, AccessGroupStoreError> {
        self.carrying_records(
            FIND_AGENTS_CARRYING_SQL,
            access_group_id,
            AgentRecord::from_row,
        )
        .await
    }

    async fn set_access_group_ids(
        &self,
        agent_id: &str,
        ids: &[String],
    ) -> Result<(), AccessGroupStoreError> {
        self.write_list(Membership::Agent, agent_id, ids).await
    }
}

impl TeamRecord {
    fn from_row(row: &Row) -> Result<Self, AccessGroupStoreError> {
        Ok(Self {
            team_id: row.try_get("team_id")?,
            team_alias: row.try_get("team_alias")?,
            access_group_ids: row.try_get("access_group_ids")?,
        })
    }
}

impl KeyRecord {
    fn from_row(row: &Row) -> Result<Self, AccessGroupStoreError> {
        Ok(Self {
            token: row.try_get("token")?,
            access_group_ids: row.try_get("access_group_ids")?,
        })
    }
}

impl AgentRecord {
    pub(super) fn from_row(row: &Row) -> Result<Self, AccessGroupStoreError> {
        Ok(Self {
            agent_id: row.try_get("agent_id")?,
            access_group_ids: row.try_get("access_group_ids")?,
        })
    }
}
