use std::collections::BTreeSet;

use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::limiter_records::{ReservationAwareIncrement, ReservationWindowIdentity};
use crate::reconciliation_operations::token_count;
use crate::token_adjustments::builders::{
    TokenReconciliation, build_project_reservation_ops, build_tpm_reconciliation_ops,
};
use crate::token_adjustments::store::TokenAdjustmentError;

use super::snapshot::ReservationSnapshot;

// which apply path the source picks for one reservation side: a plain pipeline list, a window-guarded tuple, or nothing
pub(crate) enum SideOperations {
    Empty,
    Plain(Vec<IncrementOperation>),
    ReservationAware(Vec<ReservationAwareIncrement>),
}

pub(crate) struct RefundOperations {
    pub(crate) combined: Vec<IncrementOperation>,
    pub(crate) itpm: SideOperations,
    pub(crate) otpm: SideOperations,
}

// the source branches on `reserved > 0 and reserved_window_identities`: windows make the project ops, none make the plain list
pub(crate) fn side_operations(
    scopes: &BTreeSet<(String, String)>,
    windows: &BTreeSet<ReservationWindowIdentity>,
    actual_tokens: u64,
    reserved_tokens: i64,
    window_size: i64,
) -> Result<SideOperations, TokenAdjustmentError> {
    if reserved_tokens <= 0 {
        return Ok(SideOperations::Empty);
    }
    let targets: Vec<(String, String)> = scopes.iter().cloned().collect();
    let reconciliation = TokenReconciliation {
        reserved_scopes: scopes,
        actual_tokens: token_count(actual_tokens),
        reserved_tokens,
        window_size,
    };
    if windows.is_empty() {
        let operations = build_tpm_reconciliation_ops(&targets, &reconciliation)?;
        return Ok(SideOperations::Plain(operations));
    }
    let operations = build_project_reservation_ops(&targets, &reconciliation, windows)?;
    Ok(SideOperations::ReservationAware(operations))
}

// the combined TPM refund the failure callbacks run against the stash's reserved scopes, as a plain pipeline list
pub(crate) fn combined_refund_operations(
    scopes: &BTreeSet<(String, String)>,
    reserved_tokens: i64,
    actual_tokens: u64,
    window_size: i64,
) -> Result<Vec<IncrementOperation>, TokenAdjustmentError> {
    if reserved_tokens <= 0 {
        return Ok(Vec::new());
    }
    let targets: Vec<(String, String)> = scopes.iter().cloned().collect();
    build_tpm_reconciliation_ops(
        &targets,
        &TokenReconciliation {
            reserved_scopes: scopes,
            actual_tokens: token_count(actual_tokens),
            reserved_tokens,
            window_size,
        },
    )
}

// both failure callbacks build the same three operation sets from the snapshot and settle them the same way
pub(crate) fn reserved_operations(
    snapshot: &ReservationSnapshot,
    amounts: (i64, i64, i64),
    actual: (u64, u64, u64),
    window_size: i64,
) -> Result<RefundOperations, TokenAdjustmentError> {
    Ok(RefundOperations {
        combined: combined_refund_operations(
            &snapshot.reserved_scopes,
            amounts.0,
            actual.0,
            window_size,
        )?,
        itpm: side_operations(
            &snapshot.itpm_reserved_scopes,
            &snapshot.itpm_reserved_windows,
            actual.1,
            amounts.1,
            window_size,
        )?,
        otpm: side_operations(
            &snapshot.otpm_reserved_scopes,
            &snapshot.otpm_reserved_windows,
            actual.2,
            amounts.2,
            window_size,
        )?,
    })
}
