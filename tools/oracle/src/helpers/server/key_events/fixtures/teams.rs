use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_server::key_event_sinks::ports::TeamDirectory;
use serde_json::json;

use super::board::Board;

pub(crate) struct ScriptedTeams {
    pub(crate) board: Arc<Board>,
    pub(crate) metadata: BTreeMap<String, JsonData>,
    pub(crate) database: bool,
}

#[async_trait]
impl TeamDirectory for ScriptedTeams {
    async fn team(
        &self,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamLookupError> {
        if !self.database {
            return Ok(None);
        }
        self.board.record(json!(["lookup", team_id]));
        let missing = || TeamLookupError::TeamNotFound {
            team_id: team_id.to_owned(),
        };
        let metadata = self.metadata.get(team_id).ok_or_else(missing)?;
        let row = BTreeMap::from([
            ("team_id".to_owned(), JsonData::String(team_id.to_owned())),
            ("metadata".to_owned(), metadata.clone()),
        ]);
        let team = LiteLLMTeamTableCachedObj::try_from(JsonData::Object(row))
            .ok()
            .ok_or_else(missing)?;
        Ok(Some(team))
    }
}
