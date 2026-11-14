use std::collections::BTreeSet;

use super::{
    IDENTITY_BYTES, TEXT_BYTES, check_counter, check_key, check_team, check_text_with_limit,
};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::ToolRow;
use crate::seed::FixtureSeed;

pub(super) fn validate(
    rows: &[ToolRow],
    fixture: &FixtureSeed,
    teams: &BTreeSet<&str>,
) -> Result<(), DashboardGovernanceError> {
    let mut identities = BTreeSet::new();
    let mut names = BTreeSet::new();
    for (row, tool) in rows.iter().enumerate() {
        check_text("tools", row, tool)?;
        check_key("tools", row, tool.key_index, fixture.keys.len())?;
        if let Some(team_id) = &tool.team_id {
            check_team("tools", row, team_id, teams)?;
        }
        check_counter("tools", row, "call_count", i64::from(tool.call_count))?;
        if !identities.insert(tool.tool_id.as_str()) {
            return Err(DashboardGovernanceError::DuplicateIdentity { list: "tools", row });
        }
        if !names.insert(tool.tool_name.as_str()) {
            return Err(DashboardGovernanceError::DuplicateIdentity { list: "tools", row });
        }
    }
    Ok(())
}

fn check_text(
    list: &'static str,
    row: usize,
    tool: &ToolRow,
) -> Result<(), DashboardGovernanceError> {
    check_text_with_limit(list, row, "tool_id", &tool.tool_id, IDENTITY_BYTES)?;
    check_text_with_limit(list, row, "tool_name", &tool.tool_name, IDENTITY_BYTES)?;
    for (field, value) in [
        ("origin", &tool.origin),
        ("key_alias", &tool.key_alias),
        ("user_agent", &tool.user_agent),
    ] {
        if let Some(text) = value {
            check_text_with_limit(list, row, field, text, TEXT_BYTES)?;
        }
    }
    if let Some(last_used_at) = &tool.last_used_at {
        check_text_with_limit(list, row, "last_used_at", last_used_at, IDENTITY_BYTES)?;
    }
    Ok(())
}
