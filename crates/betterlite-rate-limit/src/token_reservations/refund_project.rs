use std::collections::BTreeSet;

use crate::limiter_records::ReservationWindowIdentity;
use crate::token_adjustments::builders::{TokenReconciliation, build_project_reservation_ops};
use crate::token_adjustments::router::apply_reservation_aware_tokens;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

mod tests;

pub(crate) async fn refund_through_project_ops(
    adjustments: &TokenAdjustments,
    scopes: &[(String, String)],
    amount: i64,
    reservation_windows: &BTreeSet<ReservationWindowIdentity>,
    window_size: i64,
) -> Result<(), TokenAdjustmentError> {
    let reserved_scopes: BTreeSet<(String, String)> = scopes.iter().cloned().collect();
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved_scopes,
        actual_tokens: 0,
        reserved_tokens: amount,
        window_size,
    };
    let operations = build_project_reservation_ops(scopes, &reconciliation, reservation_windows)?;
    apply_reservation_aware_tokens(adjustments, &operations).await
}
