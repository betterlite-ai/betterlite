use std::collections::BTreeSet;

use crate::dashboard_governance::DashboardGovernanceError;

pub(super) fn check_log(
    list: &'static str,
    row: usize,
    request_id: &str,
    logs: &BTreeSet<&str>,
) -> Result<(), DashboardGovernanceError> {
    if !logs.contains(request_id) {
        return Err(DashboardGovernanceError::UnknownLog {
            list,
            row,
            request_id: request_id.to_owned(),
        });
    }
    Ok(())
}
