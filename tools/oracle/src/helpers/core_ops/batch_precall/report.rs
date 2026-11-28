use std::sync::PoisonError;

use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use betterlite_core::api::batch_precall::result::{
    BatchDiagnostic, BatchDiagnosticLevel, BatchPrecallResult,
};
use serde_json::{Value, json};

use super::Scripted;

pub(super) fn report(scripted: &Scripted, result: &BatchPrecallResult) -> Value {
    let state = scripted
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let effects = state.effects.clone();
    let held = state.held.clone();
    drop(state);
    match result {
        BatchPrecallResult::Create(create) => json!({
            "data": &create.data,
            "diagnostics": diagnostics(&create.diagnostics),
            "effects": &effects,
            "reservation": held.as_ref().map(reservation_json),
        }),
        BatchPrecallResult::CreateFailure(failure) => json!({
            "refunded": failure.refunded.as_ref().map(record_json),
            "diagnostics": diagnostics(&failure.diagnostics),
        }),
        BatchPrecallResult::Retrieve(retrieve) => json!({
            "data": &retrieve.data,
            "cached": &retrieve.cached,
            "diagnostics": diagnostics(&retrieve.diagnostics),
            "effects": &effects,
        }),
        BatchPrecallResult::TerminalRetrieve(terminal) => json!({
            "batch_id": &terminal.batch_id,
            "saved": terminal.saved,
            "refunded": terminal.refunded,
            "diagnostics": diagnostics(&terminal.diagnostics),
        }),
    }
}

fn reservation_json(reservation: &BatchEnqueuedTokenReservation) -> Value {
    json!({
        "tokens": reservation.tokens,
        "scopes": &reservation.scopes,
        "backend": reservation.backend,
        "owner": &reservation.owner,
    })
}

fn record_json(record: &ReservationRecord) -> Value {
    json!({
        "tokens": record.tokens,
        "scopes": &record.scopes,
        "backend": record.backend,
        "owner": &record.owner,
    })
}

fn diagnostics(lines: &[BatchDiagnostic]) -> Value {
    Value::Array(
        lines
            .iter()
            .map(|line| {
                let level = match line.level {
                    BatchDiagnosticLevel::Debug => "debug",
                    BatchDiagnosticLevel::Error => "error",
                };
                json!({"level": level, "message": &line.message})
            })
            .collect(),
    )
}
