use std::collections::BTreeSet;
use std::sync::PoisonError;

use crate::limiter_records::{ReservationAwareIncrement, ReservationWindowIdentity};
use crate::request_stash::{RequestStashSlot, StashState};
use crate::token_adjustments::builders::{TokenReconciliation, build_project_reservation_ops};
use crate::usage_resolution::{
    ResponseObject, ResponseUsage, Usage, aggregate_only_total_tokens, reconcile_io_usage,
    resolve_rerank_token_usage, response_usage,
};

use super::{ReconciliationError, token_count};

#[derive(Clone, Copy, Debug)]
pub enum IoTokenResponse<'request> {
    Rerank {
        tokens: Option<(u64, u64)>,
        billed_total: Option<u64>,
    },
    Response(&'request ResponseObject),
}

pub struct IoTokenReconciliation<'request> {
    pub response: IoTokenResponse<'request>,
    pub combined_usage: Option<&'request Usage>,
    pub window_size: i64,
}

pub fn io_token_reservation_ops(
    slot: &RequestStashSlot,
    call_id: Option<&str>,
    reconciliation: &IoTokenReconciliation<'_>,
) -> Result<Vec<ReservationAwareIncrement>, ReconciliationError> {
    let Some(stash) = slot.for_call(call_id) else {
        return Ok(Vec::new());
    };
    let state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
    let input_reserved = state.itpm_reserved_tokens;
    let output_reserved = state.otpm_reserved_tokens;
    if input_reserved <= 0 && output_reserved <= 0 {
        return Ok(Vec::new());
    }
    let usage = io_usage(reconciliation);
    if usage.unresolved() && !state.reservation_released {
        return Ok(Vec::new());
    }
    let counts = usage.counts(input_reserved, output_reserved);
    let window_size = reconciliation.window_size;
    if state.reservation_released
        || (state.itpm_reserved_windows.is_empty() && state.otpm_reserved_windows.is_empty())
    {
        return unguarded_sides(&state, counts, window_size);
    }
    guarded_sides(&state, counts, window_size)
}

fn unguarded_sides(
    state: &StashState,
    counts: (i64, i64),
    window_size: i64,
) -> Result<Vec<ReservationAwareIncrement>, ReconciliationError> {
    let no_windows = BTreeSet::new();
    let input = Side {
        targets: &state.itpm_reserved_scopes,
        windows: &no_windows,
        reserved_tokens: state.itpm_reserved_tokens,
        released: state.reservation_released,
    };
    let output = Side {
        targets: &state.otpm_reserved_scopes,
        windows: &no_windows,
        reserved_tokens: state.otpm_reserved_tokens,
        released: state.reservation_released,
    };
    let mut operations = input.operations(counts.0, window_size)?;
    operations.extend(output.operations(counts.1, window_size)?);
    Ok(operations)
}

fn guarded_sides(
    state: &StashState,
    counts: (i64, i64),
    window_size: i64,
) -> Result<Vec<ReservationAwareIncrement>, ReconciliationError> {
    let mut operations = Vec::new();
    if state.itpm_reserved_tokens > 0 {
        let side = Side {
            targets: &state.itpm_reserved_scopes,
            windows: &state.itpm_reserved_windows,
            reserved_tokens: state.itpm_reserved_tokens,
            released: false,
        };
        operations.extend(side.operations(counts.0, window_size)?);
    }
    if state.otpm_reserved_tokens > 0 {
        let side = Side {
            targets: &state.otpm_reserved_scopes,
            windows: &state.otpm_reserved_windows,
            reserved_tokens: state.otpm_reserved_tokens,
            released: false,
        };
        operations.extend(side.operations(counts.1, window_size)?);
    }
    Ok(operations)
}

struct Side<'state> {
    targets: &'state BTreeSet<(String, String)>,
    windows: &'state BTreeSet<ReservationWindowIdentity>,
    reserved_tokens: i64,
    released: bool,
}

impl Side<'_> {
    fn operations(
        &self,
        actual_tokens: i64,
        window_size: i64,
    ) -> Result<Vec<ReservationAwareIncrement>, ReconciliationError> {
        let no_scopes = BTreeSet::new();
        let no_windows = BTreeSet::new();
        let reserved_scopes = if self.released {
            &no_scopes
        } else {
            self.targets
        };
        let windows = if self.released {
            &no_windows
        } else {
            self.windows
        };
        let targets: Vec<(String, String)> = self.targets.iter().cloned().collect();
        let reconciliation = TokenReconciliation {
            reserved_scopes,
            actual_tokens,
            reserved_tokens: if self.released {
                0
            } else {
                self.reserved_tokens
            },
            window_size,
        };
        build_project_reservation_ops(&targets, &reconciliation, windows)
            .map_err(ReconciliationError::from)
    }
}

struct IoUsage {
    response: Option<(u64, u64)>,
    combined: Option<(u64, u64)>,
    aggregate: u64,
}

impl IoUsage {
    const fn unresolved(&self) -> bool {
        if self.response.is_some() || self.combined.is_some() {
            return false;
        }
        self.aggregate == 0
    }

    fn counts(&self, input_reserved: i64, output_reserved: i64) -> (i64, i64) {
        if let Some((input, output)) = self.response {
            return (token_count(input), token_count(output));
        }
        if let Some((input, output)) = self.combined {
            return (token_count(input), token_count(output));
        }
        if self.aggregate > 0 {
            let total = token_count(self.aggregate);
            return (total, total);
        }
        (input_reserved, output_reserved)
    }
}

fn io_usage(reconciliation: &IoTokenReconciliation<'_>) -> IoUsage {
    let response = match reconciliation.response {
        IoTokenResponse::Rerank {
            tokens,
            billed_total,
        } => resolve_rerank_token_usage(tokens, billed_total).and_then(resolved_counts),
        IoTokenResponse::Response(response_object) => {
            resolved_counts(reconcile_io_usage(response_usage(response_object)))
        }
    };
    let combined = reconciliation
        .combined_usage
        .and_then(|usage| resolved_counts(reconcile_io_usage(Some(ResponseUsage::Usage(usage)))));
    let response_total = match reconciliation.response {
        IoTokenResponse::Rerank { .. } => 0,
        IoTokenResponse::Response(response_object) => {
            aggregate_only_total_tokens(response_usage(response_object))
        }
    };
    let combined_total =
        aggregate_only_total_tokens(reconciliation.combined_usage.map(ResponseUsage::Usage));
    let aggregate = if response_total > 0 {
        response_total
    } else {
        combined_total
    };
    IoUsage {
        response,
        combined,
        aggregate,
    }
}

const fn resolved_counts(counts: (u64, u64, bool)) -> Option<(u64, u64)> {
    if counts.2 {
        return Some((counts.0, counts.1));
    }
    None
}
