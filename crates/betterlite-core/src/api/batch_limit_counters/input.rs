use std::collections::BTreeSet;

use serde::Deserialize;

use crate::api::batch_enqueued_reserve::BatchEnqueuedTokenScope;
use crate::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::request_destination::RequestData;

use super::records::{
    BatchCounterStatus, BatchEnqueuedLimitExceeded, BatchFileUsage, BatchReservationWindow,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchLimitCountersInput {
    pub principal: BatchLimitPrincipal,
    pub calls: Vec<BatchLimitCounterCall>,
}

// One owned callable of the slice with the typed facts it reads
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub enum BatchLimitCounterCall {
    ReserveBatchEnqueuedTokens {
        data: RequestData,
        batch_usage: BatchFileUsage,
        scopes: Vec<BatchEnqueuedTokenScope>,
    },
    RaiseEnqueuedLimitError {
        over_limit: BatchEnqueuedLimitExceeded,
        data: RequestData,
        batch_usage: BatchFileUsage,
    },
    RaiseRateLimitError {
        status: BatchCounterStatus,
        descriptors: Vec<BatchLimitDescriptor>,
        batch_usage: BatchFileUsage,
        // the source declares a plain str here, so an unknown dimension still reaches the headers
        limit_type: String,
        #[serde(default)]
        requested_model: Option<String>,
        #[serde(default)]
        window_start: Option<i64>,
    },
    CheckAndIncrementBatchCounters {
        data: RequestData,
        batch_usage: BatchFileUsage,
        // absent or null is the source's descriptors=None, which rebuilds them through the limiter
        #[serde(default)]
        descriptors: Option<Vec<BatchLimitDescriptor>>,
    },
    ReadTpdWindowStart {
        status: BatchCounterStatus,
    },
    BuildTpdRefundOps {
        descriptors: Vec<BatchLimitDescriptor>,
        tokens: i64,
        #[serde(default)]
        reservation_windows: BTreeSet<BatchReservationWindow>,
    },
}
