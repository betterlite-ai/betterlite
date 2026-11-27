use crate::api::batch_enqueued_refund::record::ReservationRecord;
use crate::api::batch_enqueued_refund::{
    BatchEnqueuedRefundInput, BatchEnqueuedRefundResult, PostCallOutcome,
};

use super::input::BatchTerminalCall;
use super::ports::{BatchPrecallStash, BatchRefundSink, BatchTokenApplier};
use super::result::{BatchCreateFailureResult, BatchDiagnostic, BatchTerminalResult, error};

const FAILURE_PREFIX: &str = "Error releasing TPM reservation on post-call failure: ";
const POST_CALL_PREFIX: &str = "Error in batch enqueued-token post-call hook: ";

pub(crate) struct RefundHook<'a, Stash, Refunds, Applier> {
    pub(crate) stash: &'a Stash,
    pub(crate) refunds: &'a Refunds,
    pub(crate) applier: &'a Applier,
}

impl<Stash, Refunds, Applier> RefundHook<'_, Stash, Refunds, Applier>
where
    Stash: BatchPrecallStash,
    Refunds: BatchRefundSink,
    Applier: BatchTokenApplier,
{
    // async_post_call_failure_hook's batch part: refund the stashed reservation, then apply the TPD refunds; a failure is logged and stops the rest
    pub(crate) async fn create_failure(&self) -> BatchCreateFailureResult {
        let mut diagnostics = Vec::new();
        let mut refunded = None;
        if let Some(reservation) = self.stash.take_reservation() {
            let record = ReservationRecord::from(&reservation);
            let input = BatchEnqueuedRefundInput::Refund {
                reservation: record.clone(),
            };
            // the store's refund either releases the reservation or fails, and a returned call cleared the stash in the source
            if let Err(failure) = self.refunds.run(&input).await {
                diagnostics.push(error(format!("{FAILURE_PREFIX}{failure}")));
                return BatchCreateFailureResult {
                    refunded: None,
                    applied: Vec::new(),
                    diagnostics,
                };
            }
            refunded = Some(record);
        }
        let operations = self.stash.take_refund_operations();
        let applied = if operations.is_empty() {
            Vec::new()
        } else {
            match self.applier.apply(&operations).await {
                Ok(()) => operations,
                Err(failure) => {
                    diagnostics.push(error(format!("{FAILURE_PREFIX}{failure}")));
                    return BatchCreateFailureResult {
                        refunded,
                        applied: Vec::new(),
                        diagnostics,
                    };
                }
            }
        };
        BatchCreateFailureResult {
            refunded,
            applied,
            diagnostics,
        }
    }

    // _handle_batch_enqueued_post_call: save the request's reservation under the provider batch id, then refund it at a terminal status
    pub(crate) async fn terminal(&self, call: &BatchTerminalCall) -> BatchTerminalResult {
        let pending = self
            .stash
            .take_reservation()
            .map(|reservation| ReservationRecord::from(&reservation));
        let input = BatchEnqueuedRefundInput::PostCall {
            batch_id: call.batch_id.clone(),
            status: call.status.clone(),
            pending,
        };
        let mut diagnostics = Vec::new();
        match self.refunds.run(&input).await {
            Ok(BatchEnqueuedRefundResult::PostCall(outcome)) => {
                terminal_result(outcome, diagnostics)
            }
            // the store answers a PostCall input with PostCall; the rest would be a sink wired to another entry
            Ok(
                BatchEnqueuedRefundResult::Saved
                | BatchEnqueuedRefundResult::Refunded
                | BatchEnqueuedRefundResult::Popped(Some(_))
                | BatchEnqueuedRefundResult::Popped(None),
            ) => terminal_result(
                PostCallOutcome {
                    batch_id: call.batch_id.clone(),
                    saved: false,
                    refunded: false,
                },
                diagnostics,
            ),
            Err(failure) => {
                diagnostics.push(error(format!("{POST_CALL_PREFIX}{failure}")));
                terminal_result(
                    PostCallOutcome {
                        batch_id: call.batch_id.clone(),
                        saved: false,
                        refunded: false,
                    },
                    diagnostics,
                )
            }
        }
    }
}

fn terminal_result(
    outcome: PostCallOutcome,
    diagnostics: Vec<BatchDiagnostic>,
) -> BatchTerminalResult {
    BatchTerminalResult {
        batch_id: outcome.batch_id,
        saved: outcome.saved,
        refunded: outcome.refunded,
        diagnostics,
    }
}
