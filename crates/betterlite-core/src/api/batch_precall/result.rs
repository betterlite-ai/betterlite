use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;

use crate::api::batch_enqueued_refund::record::ReservationRecord;
use crate::api::batch_input_accounting::error::BatchHttpError;
use crate::api::batch_limit_counters::records::BatchTpdRefundOperation;
use crate::api::request_destination::RequestData;

#[derive(Debug, Clone, PartialEq)]
pub enum BatchPrecallResult {
    Create(BatchCreateResult),
    CreateFailure(BatchCreateFailureResult),
    Retrieve(BatchRetrieveResult),
    TerminalRetrieve(BatchTerminalResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchDiagnosticLevel {
    Debug,
    Error,
}

// One line the source writes through verbose_proxy_logger, in the order it writes them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchDiagnostic {
    pub level: BatchDiagnosticLevel,
    pub message: String,
}

// The create pre-call's answer: the request data the route continues with, and what the hook did to it
#[derive(Debug, Clone, PartialEq)]
pub struct BatchCreateResult {
    pub data: RequestData,
    pub outcome: BatchCreateOutcome,
    pub diagnostics: Vec<BatchDiagnostic>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BatchCreateOutcome {
    Passed,
    Rejected(Box<ProxyRateLimitError>),
    Raised(BatchHttpError),
}

// The retrieve pre-call's answer: the warm-up's diagnostics, the cache read, and the HTTPException the source re-raises
#[derive(Debug, Clone, PartialEq)]
pub struct BatchRetrieveResult {
    pub data: RequestData,
    pub cached: Option<Json>,
    pub raised: Option<BatchHttpError>,
    pub diagnostics: Vec<BatchDiagnostic>,
    pub printed: Vec<String>,
}

// The create failure's answer: the reservation handed back to the enqueued-token store, and the TPD refunds applied
#[derive(Debug, Clone, PartialEq)]
pub struct BatchCreateFailureResult {
    pub refunded: Option<ReservationRecord>,
    pub applied: Vec<BatchTpdRefundOperation>,
    pub diagnostics: Vec<BatchDiagnostic>,
}

// The terminal retrieve's answer, read from the refund store's post-call outcome
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchTerminalResult {
    pub batch_id: String,
    pub saved: bool,
    pub refunded: bool,
    pub diagnostics: Vec<BatchDiagnostic>,
}

pub(crate) fn debug(message: String) -> BatchDiagnostic {
    BatchDiagnostic {
        level: BatchDiagnosticLevel::Debug,
        message,
    }
}

pub(crate) fn error(message: String) -> BatchDiagnostic {
    BatchDiagnostic {
        level: BatchDiagnosticLevel::Error,
        message,
    }
}
