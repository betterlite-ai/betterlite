use betterlite_context::call_context::BillingPolicy;

use super::error::DispatchError;
use crate::operation::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

pub struct CallRecord<'call> {
    pub entry: Entry,
    pub call_id: &'call str,
    pub billing: BillingPolicy,
    pub privacy: bool,
    pub outcome: CallOutcome,
    pub error: Option<&'call DispatchError>,
    pub message: Option<&'call str>,
}

pub trait CallLog: Send + Sync {
    fn record(&self, record: CallRecord<'_>);
}

pub struct TracingCallLog;

impl CallLog for TracingCallLog {
    fn record(&self, record: CallRecord<'_>) {
        let entry = <&str>::from(record.entry);
        let call_id = record.call_id;
        match record.outcome {
            CallOutcome::Succeeded => tracing::debug!(entry, call_id, "call finished"),
            CallOutcome::Cancelled => {
                tracing::info!(entry, call_id, "client disconnected, call cancelled");
            }
            CallOutcome::Failed => log_failure(&record, entry, call_id),
        }
    }
}

fn log_failure(record: &CallRecord<'_>, entry: &str, call_id: &str) {
    let message = record
        .message
        .map(str::to_owned)
        .or_else(|| record.error.map(ToString::to_string))
        .unwrap_or_default();
    tracing::error!(entry, call_id, error = message, "call failed");
}
