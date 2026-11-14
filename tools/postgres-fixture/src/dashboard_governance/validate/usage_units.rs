use std::collections::BTreeSet;

use super::{
    IDENTITY_BYTES, TEXT_BYTES, check_counter, check_key, check_team, check_text_with_limit,
};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::UsageUnitRow;
use crate::seed::FixtureSeed;

pub(super) fn validate(
    rows: &[UsageUnitRow],
    fixture: &FixtureSeed,
    teams: &BTreeSet<&str>,
) -> Result<(), DashboardGovernanceError> {
    let mut identities = BTreeSet::new();
    for (row, unit) in rows.iter().enumerate() {
        check_text_limits(row, unit)?;
        check_key("usage_units", row, unit.key_index, fixture.keys.len())?;
        check_team("usage_units", row, &unit.team_id, teams)?;
        check_counter("usage_units", row, "units", unit.units)?;
        check_counter("usage_units", row, "untracked_units", unit.untracked_units)?;
        let identity = (
            unit.guardrail_id.as_str(),
            unit.date.as_str(),
            unit.team_id.as_str(),
            unit.key_index,
            unit.usage_unit.as_str(),
        );
        if !identities.insert(identity) {
            return Err(DashboardGovernanceError::DuplicateIdentity {
                list: "usage_units",
                row,
            });
        }
    }
    Ok(())
}

fn check_text_limits(row: usize, unit: &UsageUnitRow) -> Result<(), DashboardGovernanceError> {
    check_text_with_limit(
        "usage_units",
        row,
        "guardrail_id",
        &unit.guardrail_id,
        IDENTITY_BYTES,
    )?;
    check_text_with_limit("usage_units", row, "date", &unit.date, IDENTITY_BYTES)?;
    check_text_with_limit("usage_units", row, "team_id", &unit.team_id, IDENTITY_BYTES)?;
    check_text_with_limit(
        "usage_units",
        row,
        "usage_unit",
        &unit.usage_unit,
        TEXT_BYTES,
    )
}
