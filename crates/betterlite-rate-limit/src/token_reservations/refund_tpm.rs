use std::collections::BTreeSet;

use crate::token_adjustments::builders::{TokenReconciliation, build_tpm_reconciliation_ops};
use crate::token_adjustments::direct::increment_tokens_keeping_ttl;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

mod tests;

pub(crate) async fn refund_through_tpm_ops(
    adjustments: &TokenAdjustments,
    scopes: &[(String, String)],
    amount: i64,
    window_size: i64,
) -> Result<(), TokenAdjustmentError> {
    let reserved_scopes: BTreeSet<(String, String)> = scopes.iter().cloned().collect();
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved_scopes,
        actual_tokens: 0,
        reserved_tokens: amount,
        window_size,
    };
    let operations = build_tpm_reconciliation_ops(scopes, &reconciliation)?;
    increment_tokens_keeping_ttl(adjustments, &operations).await;
    Ok(())
}
