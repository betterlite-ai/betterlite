mod batches;
mod team;

use std::collections::BTreeMap;

use crate::budget_permission_rows::Timestamp;
use crate::key_repository::table::TokenValue;
use crate::team_rows::team::LiteLLMTeamTable;

use super::columns::TableColumn;

// Prisma's @updatedAt writes the current time on every update that does not set it explicitly
pub(super) fn stamp_updated_at<Column>(payload: &mut BTreeMap<Column, TokenValue>, now: Timestamp)
where
    Column: TableColumn,
{
    let Some(column) = Column::UPDATED_AT else {
        return;
    };
    if payload.is_empty() || payload.contains_key(&column) {
        return;
    }
    payload.insert(column, TokenValue::Timestamp(now));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamUpdateResult {
    pub team_id: String,
    pub data: LiteLLMTeamTable,
}
