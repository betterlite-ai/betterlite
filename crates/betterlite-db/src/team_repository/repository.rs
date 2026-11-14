use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::{DbSession, PostgresStore};
use crate::team_repository::decoder::{
    TeamRecord, TeamRecordError, convert_record, decode_locked_roster,
};
use crate::team_rows::decoder::parse_data;
use crate::team_rows::member::Member;
use crate::team_rows::team::LiteLLMTeamTable;
use tokio_postgres::Row;

const FIND_BY_ID_SQL: &str =
    r#"SELECT row_to_json(team)::TEXT FROM "LiteLLM_TeamTable" team WHERE team_id = $1"#;
const FIND_BY_ORGANIZATION_SQL: &str =
    r#"SELECT row_to_json(team)::TEXT FROM "LiteLLM_TeamTable" team WHERE organization_id = $1"#;
// no FOR UPDATE: the caller's advisory lock serializes the roster, and a team-row-first lock deadlocks access-group endpoints
const MEMBERS_WITH_ROLES_SQL: &str =
    r#"SELECT members_with_roles::TEXT FROM "LiteLLM_TeamTable" WHERE team_id = $1"#;

#[derive(Debug, thiserror::Error)]
pub enum TeamQueryError {
    #[error("the team table query failed")]
    Database {
        #[from]
        source: DbError,
    },
    #[error("a stored team row cannot be decoded")]
    Record {
        #[from]
        source: TeamRecordError,
    },
}

// the two pooled handles a repository can read through, which LiteLLM exposes as client.db and client.writer_db
#[derive(Clone, Copy)]
pub struct TeamDatabase<'database> {
    pub db: &'database PostgresStore,
    pub writer_db: &'database PostgresStore,
}

pub struct TeamRepository<'database> {
    pub database: TeamDatabase<'database>,
    pub use_writer: bool,
}

pub struct TeamTable<'database> {
    pub store: &'database PostgresStore,
}

impl<'database> TeamRepository<'database> {
    #[must_use]
    pub const fn table(&self) -> TeamTable<'database> {
        TeamTable {
            store: if self.use_writer {
                self.database.writer_db
            } else {
                self.database.db
            },
        }
    }

    pub async fn find_by_id(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTable>, TeamQueryError> {
        let record = self.table().find_unique(team_id).await?;
        record
            .as_ref()
            .map(convert_record)
            .transpose()
            .map_err(TeamQueryError::from)
    }

    pub async fn find_by_organization_id(
        &self,
        organization_id: &str,
    ) -> Result<Vec<LiteLLMTeamTable>, TeamQueryError> {
        let records = self
            .table()
            .find_many_by_organization(organization_id)
            .await?;
        let mut teams = Vec::with_capacity(records.len());
        for record in &records {
            teams.push(convert_record(record)?);
        }
        Ok(teams)
    }

    pub async fn get_members_with_roles_locked(
        tx: &DbSession<'_>,
        team_id: &str,
    ) -> Result<Option<Vec<Member>>, TeamQueryError> {
        let row = tx
            .query_opt(MEMBERS_WITH_ROLES_SQL, &[&team_id])
            .await
            .map_err(backend)?;
        let Some(found) = row else {
            return Ok(None);
        };
        let stored: String = found.try_get(0).map_err(backend)?;
        let value =
            parse_data(stored.as_bytes()).map_err(|source| TeamRecordError::Record { source })?;
        decode_locked_roster(&value)
            .map(Some)
            .map_err(TeamQueryError::from)
    }
}

impl TeamTable<'_> {
    pub async fn find_unique(&self, team_id: &str) -> Result<Option<TeamRecord>, TeamQueryError> {
        let client = self.store.session().await?;
        let row = client
            .query_opt(FIND_BY_ID_SQL, &[&team_id])
            .await
            .map_err(backend)?;
        let Some(found) = row else {
            return Ok(None);
        };
        row_record(&found).map(Some)
    }

    pub async fn find_many_by_organization(
        &self,
        organization_id: &str,
    ) -> Result<Vec<TeamRecord>, TeamQueryError> {
        let client = self.store.session().await?;
        let rows = client
            .query(FIND_BY_ORGANIZATION_SQL, &[&organization_id])
            .await
            .map_err(backend)?;
        rows.iter().map(row_record).collect()
    }
}

fn row_record(row: &Row) -> Result<TeamRecord, TeamQueryError> {
    let text: String = row.try_get(0).map_err(backend)?;
    let record = TeamRecord::parse(text.as_bytes())?;
    record.ok_or(TeamQueryError::Record {
        source: TeamRecordError::NotAnObject,
    })
}
