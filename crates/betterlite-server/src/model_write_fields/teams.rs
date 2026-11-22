use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::repository::{TeamDatabase, TeamQueryError, TeamRepository};
use betterlite_db::team_rows::team::LiteLLMTeamTable;

#[async_trait]
pub trait TeamRows: Send + Sync {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, TeamQueryError>;
}

// _repo_team_table pins the read to the writer, so a team created a moment ago is already there
pub struct WriterTeamRows {
    pub writer: Arc<PostgresStore>,
}

#[async_trait]
impl TeamRows for WriterTeamRows {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, TeamQueryError> {
        let repository = TeamRepository {
            database: TeamDatabase {
                db: self.writer.as_ref(),
                writer_db: self.writer.as_ref(),
            },
            use_writer: true,
        };
        repository.find_by_id(team_id).await
    }
}
