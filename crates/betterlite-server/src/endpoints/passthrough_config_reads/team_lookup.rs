use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::TeamDatabase;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_repository::repository::TeamRepository;

use super::ports::TeamLookup;

const METADATA_FIELD: &str = "metadata";

pub struct DatabaseTeamLookup {
    pub store: Arc<PostgresStore>,
}

#[async_trait]
impl TeamLookup for DatabaseTeamLookup {
    // LiteLLM reads the team row through the reader connection, not the writer
    async fn find_team_metadata(&self, team_id: &str) -> Result<Option<JsonData>, TeamQueryError> {
        let teams = TeamRepository {
            database: TeamDatabase {
                db: self.store.as_ref(),
                writer_db: self.store.as_ref(),
            },
            use_writer: false,
        };
        let Some(record) = teams.table().find_unique(team_id).await? else {
            return Ok(None);
        };
        let metadata = record
            .fields
            .get(METADATA_FIELD)
            .cloned()
            .unwrap_or(JsonData::Null);
        Ok(Some(metadata))
    }
}
