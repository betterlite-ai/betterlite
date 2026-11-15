use std::collections::BTreeSet;

use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;

use crate::limiter_records::{
    RateLimitType, ReservationAwareIncrement, ReservationWindowIdentity, create_rate_limit_keys,
};

use super::store::{TokenAdjustmentError, token_float};

pub struct TokenReconciliation<'scopes> {
    pub reserved_scopes: &'scopes BTreeSet<(String, String)>,
    pub actual_tokens: i64,
    pub reserved_tokens: i64,
    pub window_size: i64,
}

impl TokenReconciliation<'_> {
    const fn delta(&self, reserved: bool) -> i64 {
        if reserved {
            return self.actual_tokens.saturating_sub(self.reserved_tokens);
        }
        self.actual_tokens
    }
}

pub fn build_tpm_reconciliation_ops(
    targets: &[(String, String)],
    reconciliation: &TokenReconciliation<'_>,
) -> Result<Vec<IncrementOperation>, TokenAdjustmentError> {
    let mut operations = Vec::new();
    for target in targets {
        let reserved = reconciliation.reserved_scopes.contains(target);
        let increment = reconciliation.delta(reserved);
        if increment == 0 {
            continue;
        }
        operations.push(IncrementOperation {
            key: create_rate_limit_keys(&target.0, &target.1, RateLimitType::Tokens),
            increment_value: token_float(increment)?,
            ttl: Some(reconciliation.window_size),
        });
    }
    Ok(operations)
}

pub fn build_project_reservation_op(
    target: &(String, String),
    reconciliation: &TokenReconciliation<'_>,
    reservation_window_identities: &BTreeSet<ReservationWindowIdentity>,
) -> Result<Option<ReservationAwareIncrement>, TokenAdjustmentError> {
    let reserved = reconciliation.reserved_scopes.contains(target);
    let increment = reconciliation.delta(reserved);
    if increment == 0 {
        return Ok(None);
    }
    let counter_key = create_rate_limit_keys(&target.0, &target.1, RateLimitType::Tokens);
    let mut reservation = ReservationAwareIncrement {
        operation: IncrementOperation {
            key: counter_key.clone(),
            increment_value: token_float(increment)?,
            ttl: Some(reconciliation.window_size),
        },
        window_key: None,
        expected_window_start: None,
        reservation_backend: None,
    };
    let window_identity = if reserved {
        reservation_window_identities
            .iter()
            .find(|identity| identity.counter_key == counter_key)
    } else {
        None
    };
    let Some(identity) = window_identity else {
        return Ok(Some(reservation));
    };
    reservation.window_key = Some(format!("{{{}:{}}}:window", target.0, target.1));
    reservation.expected_window_start = Some(identity.window_start.clone());
    reservation.reservation_backend = Some(identity.backend);
    Ok(Some(reservation))
}

pub fn build_project_reservation_ops(
    targets: &[(String, String)],
    reconciliation: &TokenReconciliation<'_>,
    reservation_window_identities: &BTreeSet<ReservationWindowIdentity>,
) -> Result<Vec<ReservationAwareIncrement>, TokenAdjustmentError> {
    let mut operations = Vec::new();
    for target in targets {
        if let Some(reservation) =
            build_project_reservation_op(target, reconciliation, reservation_window_identities)?
        {
            operations.push(reservation);
        }
    }
    Ok(operations)
}
