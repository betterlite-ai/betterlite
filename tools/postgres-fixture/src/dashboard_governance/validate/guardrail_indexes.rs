use std::collections::BTreeSet;

use super::log::check_log;
use super::{IDENTITY_BYTES, check_text_with_limit};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::GuardrailIndexRow;

pub(super) fn validate(
    guardrail_index: &[GuardrailIndexRow],
    logs: &BTreeSet<&str>,
) -> Result<(), DashboardGovernanceError> {
    let mut identities = BTreeSet::new();
    for (row, index) in guardrail_index.iter().enumerate() {
        check_text_limits(row, index)?;
        check_log("guardrail_index", row, &index.request_id, logs)?;
        if !identities.insert((index.request_id.as_str(), index.guardrail_id.as_str())) {
            return Err(DashboardGovernanceError::DuplicateIdentity {
                list: "guardrail_index",
                row,
            });
        }
    }
    Ok(())
}

fn check_text_limits(
    row: usize,
    index: &GuardrailIndexRow,
) -> Result<(), DashboardGovernanceError> {
    check_text_with_limit(
        "guardrail_index",
        row,
        "request_id",
        &index.request_id,
        IDENTITY_BYTES,
    )?;
    check_text_with_limit(
        "guardrail_index",
        row,
        "guardrail_id",
        &index.guardrail_id,
        IDENTITY_BYTES,
    )?;
    if let Some(policy_id) = &index.policy_id {
        check_text_with_limit(
            "guardrail_index",
            row,
            "policy_id",
            policy_id,
            IDENTITY_BYTES,
        )?;
    }
    check_text_with_limit(
        "guardrail_index",
        row,
        "start_time",
        &index.start_time,
        IDENTITY_BYTES,
    )
}
