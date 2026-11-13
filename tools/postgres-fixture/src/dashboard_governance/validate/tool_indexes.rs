use std::collections::BTreeSet;

use super::log::check_log;
use super::{IDENTITY_BYTES, check_text_with_limit};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::ToolIndexRow;

pub(super) fn validate(
    tool_index: &[ToolIndexRow],
    logs: &BTreeSet<&str>,
) -> Result<(), DashboardGovernanceError> {
    let mut identities = BTreeSet::new();
    for (row, index) in tool_index.iter().enumerate() {
        check_text_with_limit(
            "tool_index",
            row,
            "request_id",
            &index.request_id,
            IDENTITY_BYTES,
        )?;
        check_text_with_limit(
            "tool_index",
            row,
            "tool_name",
            &index.tool_name,
            IDENTITY_BYTES,
        )?;
        check_text_with_limit(
            "tool_index",
            row,
            "start_time",
            &index.start_time,
            IDENTITY_BYTES,
        )?;
        check_log("tool_index", row, &index.request_id, logs)?;
        if !identities.insert((index.request_id.as_str(), index.tool_name.as_str())) {
            return Err(DashboardGovernanceError::DuplicateIdentity {
                list: "tool_index",
                row,
            });
        }
    }
    Ok(())
}
