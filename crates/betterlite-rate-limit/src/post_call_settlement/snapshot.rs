use std::collections::BTreeSet;
use std::sync::PoisonError;

use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;

use crate::limiter_records::{
    RateLimitStatus, ReservationAwareIncrement, ReservationWindowIdentity,
};
use crate::request_stash::RequestStash;

// one locked read of the reservation fields the failure callbacks settle against
pub(crate) struct ReservationSnapshot {
    pub(crate) reservation_released: bool,
    pub(crate) reserved_tokens: i64,
    pub(crate) reserved_scopes: BTreeSet<(String, String)>,
    pub(crate) itpm_reserved_tokens: i64,
    pub(crate) itpm_reserved_scopes: BTreeSet<(String, String)>,
    pub(crate) itpm_reserved_windows: BTreeSet<ReservationWindowIdentity>,
    pub(crate) otpm_reserved_tokens: i64,
    pub(crate) otpm_reserved_scopes: BTreeSet<(String, String)>,
    pub(crate) otpm_reserved_windows: BTreeSet<ReservationWindowIdentity>,
}

pub(crate) fn stash_snapshot(stash: &RequestStash) -> ReservationSnapshot {
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    ReservationSnapshot {
        reservation_released: state.reservation_released,
        reserved_tokens: state.reserved_tokens,
        reserved_scopes: state.reserved_scopes.clone(),
        itpm_reserved_tokens: state.itpm_reserved_tokens,
        itpm_reserved_scopes: state.itpm_reserved_scopes.clone(),
        itpm_reserved_windows: state.itpm_reserved_windows.clone(),
        otpm_reserved_tokens: state.otpm_reserved_tokens,
        otpm_reserved_scopes: state.otpm_reserved_scopes.clone(),
        otpm_reserved_windows: state.otpm_reserved_windows.clone(),
    }
}

// LiteLLM returns early from the mirror when the stashed response carries no statuses
pub(crate) fn rate_limit_statuses(stash: Option<&RequestStash>) -> Option<Vec<RateLimitStatus>> {
    let response = stash?
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .rate_limit_response
        .clone()?;
    if response.statuses.is_empty() {
        return None;
    }
    Some(response.statuses)
}

pub(crate) fn batch_enqueued_reservation(
    stash: &RequestStash,
) -> Option<BatchEnqueuedTokenReservation> {
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .batch_enqueued_reservation
        .clone()
}

pub(crate) fn clear_batch_enqueued_reservation(stash: &RequestStash) {
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .batch_enqueued_reservation = None;
}

pub(crate) fn batch_tpd_refund_ops(stash: &RequestStash) -> Vec<ReservationAwareIncrement> {
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .batch_tpd_refund_ops
        .clone()
}

pub(crate) fn clear_batch_tpd_refund_ops(stash: &RequestStash) {
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .batch_tpd_refund_ops = Vec::new();
}

pub(crate) fn mark_reservation_released(stash: &RequestStash) {
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .reservation_released = true;
}
