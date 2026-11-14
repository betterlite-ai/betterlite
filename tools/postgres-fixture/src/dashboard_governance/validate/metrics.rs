use std::collections::BTreeSet;

use super::{IDENTITY_BYTES, check_counter, check_text_with_limit};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::GuardrailMetricRow;

pub(super) fn validate(rows: &[GuardrailMetricRow]) -> Result<(), DashboardGovernanceError> {
    let mut identities = BTreeSet::new();
    for (row, metric) in rows.iter().enumerate() {
        check_text_with_limit(
            "metrics",
            row,
            "guardrail_id",
            &metric.guardrail_id,
            IDENTITY_BYTES,
        )?;
        check_text_with_limit("metrics", row, "date", &metric.date, IDENTITY_BYTES)?;
        for (field, value) in [
            ("requests_evaluated", metric.requests_evaluated),
            ("passed_count", metric.passed_count),
            ("blocked_count", metric.blocked_count),
            ("flagged_count", metric.flagged_count),
        ] {
            check_counter("metrics", row, field, value)?;
        }
        if !identities.insert((metric.guardrail_id.as_str(), metric.date.as_str())) {
            return Err(DashboardGovernanceError::DuplicateIdentity {
                list: "metrics",
                row,
            });
        }
    }
    Ok(())
}
