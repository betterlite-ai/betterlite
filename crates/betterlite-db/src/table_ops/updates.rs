mod batches;
mod team;

use crate::team_rows::team::LiteLLMTeamTable;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamUpdateResult {
    pub team_id: String,
    pub data: LiteLLMTeamTable,
}
