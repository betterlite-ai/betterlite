use std::sync::Arc;

use betterlite_context::call_context::BillingPolicy;

use super::error::DispatchError;
use super::log::{CallLog, CallOutcome, CallRecord};
use crate::operation::Entry;

pub struct Terminal {
    pub(crate) log: Arc<dyn CallLog>,
    pub(crate) entry: Entry,
    pub(crate) call_id: String,
    pub(crate) billing: BillingPolicy,
    pub(crate) privacy: bool,
}

impl Terminal {
    pub(crate) fn record(
        self,
        outcome: CallOutcome,
        error: Option<&DispatchError>,
        message: Option<&str>,
    ) {
        self.log.record(CallRecord {
            entry: self.entry,
            call_id: &self.call_id,
            billing: self.billing,
            privacy: self.privacy,
            outcome,
            error,
            message,
        });
    }
}
