use betterlite_server::model_visibility::testing::{Row as SnapshotRow, Snapshot};

use super::error::Failure;
use super::input::{Input, RouterCall};

// Router(model_list=..., model_group_alias=...): the case's rows and aliases as the listing filters read them, absent when the call passes no router.
pub(crate) fn snapshot(input: &Input) -> Result<Option<Snapshot>, Failure> {
    if matches!(input.arguments.router, RouterCall::Missing) {
        return Ok(None);
    }
    let rows = declared_rows(input);
    let mut fixture = Snapshot::build(&rows, &input.model_group_alias)?;
    if matches!(input.arguments.router, RouterCall::BrokenPattern) {
        fixture = fixture.with_unbacked_pattern()?;
    }
    Ok(Some(fixture))
}

// One row per deployment the case declares, borrowing the case's own values.
pub(crate) fn declared_rows(input: &Input) -> Vec<SnapshotRow<'_>> {
    input
        .model_list
        .iter()
        .map(|row| SnapshotRow {
            model_name: &row.model_name,
            litellm_params: &row.litellm_params,
            model_info: &row.model_info,
        })
        .collect()
}
