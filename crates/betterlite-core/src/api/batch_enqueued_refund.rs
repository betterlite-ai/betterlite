use std::sync::{Arc, Mutex};

use betterlite_context::local_ttl_cache::CacheError;
use serde::Deserialize;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_enqueued_reserve::provider::canonical_provider_batch_id;
use crate::api::batch_enqueued_reserve::store::BatchEnqueuedReserveError;
use crate::api::batch_enqueued_reserve::{BatchEnqueuedTokenSettings, Monotonic};
use record::ReservationRecord;
use store::{EnqueuedLocalStore, ReservationRecordScripts};

pub mod pop;
pub mod record;
pub mod refund;
pub mod save;
pub mod store;
mod tests;

// the batch statuses that release the reservation, read after lowercasing the provider's status
pub const BATCH_ENQUEUED_REFUND_STATUSES: [&str; 6] = [
    "completed",
    "complete",
    "failed",
    "expired",
    "cancelled",
    "cancelling",
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BatchEnqueuedRefundInput {
    PostCall {
        batch_id: String,
        status: String,
        #[serde(default)]
        pending: Option<ReservationRecord>,
    },
    Refund {
        reservation: ReservationRecord,
    },
    Save {
        batch_id: String,
        reservation: ReservationRecord,
    },
    Pop {
        batch_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BatchEnqueuedRefundResult {
    PostCall(PostCallOutcome),
    Saved,
    Popped(Option<ReservationRecord>),
    Refunded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostCallOutcome {
    pub batch_id: String,
    pub saved: bool,
    pub refunded: bool,
}

#[derive(Debug, Error)]
pub enum BatchEnqueuedRefundError {
    #[error("the local enqueued-token store failed")]
    Store(#[source] Box<BatchEnqueuedReserveError>),
    #[error("the local enqueued-token cache failed")]
    Cache(#[source] Box<CacheError>),
    #[error("the enqueued-token reservation record could not be encoded")]
    Encode {
        #[source]
        source: Box<serde_json::Error>,
    },
}

impl From<BatchEnqueuedReserveError> for BatchEnqueuedRefundError {
    fn from(source: BatchEnqueuedReserveError) -> Self {
        Self::Store(Box::new(source))
    }
}

impl From<CacheError> for BatchEnqueuedRefundError {
    fn from(source: CacheError) -> Self {
        Self::Cache(Box::new(source))
    }
}

// the store methods the source splits across refund, save_reservation and pop_reservation, over one set of injected dependencies
pub struct EnqueuedTokenRefund<Scripts, Store> {
    pub clock: Monotonic,
    pub scripts: Option<Scripts>,
    pub store: Arc<Mutex<Store>>,
    pub owner: String,
    pub settings: BatchEnqueuedTokenSettings,
    pub limits: ApiLimits,
}

impl<Scripts, Store> EnqueuedTokenRefund<Scripts, Store>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    pub async fn run(
        &self,
        input: &BatchEnqueuedRefundInput,
    ) -> Result<BatchEnqueuedRefundResult, BatchEnqueuedRefundError> {
        match input {
            BatchEnqueuedRefundInput::PostCall {
                batch_id,
                status,
                pending,
            } => Ok(BatchEnqueuedRefundResult::PostCall(
                self.post_call(batch_id, status, pending.as_ref()).await?,
            )),
            BatchEnqueuedRefundInput::Refund { reservation } => {
                refund::refund(self, reservation).await?;
                Ok(BatchEnqueuedRefundResult::Refunded)
            }
            BatchEnqueuedRefundInput::Save {
                batch_id,
                reservation,
            } => {
                save::save_reservation(self, batch_id, reservation).await?;
                Ok(BatchEnqueuedRefundResult::Saved)
            }
            BatchEnqueuedRefundInput::Pop { batch_id } => Ok(BatchEnqueuedRefundResult::Popped(
                pop::pop_reservation(self, batch_id).await?,
            )),
        }
    }

    // the source's rate-limit post-call hook: persist the request's reservation, then on a terminal status consume and refund it
    async fn post_call(
        &self,
        batch_id: &str,
        status: &str,
        pending: Option<&ReservationRecord>,
    ) -> Result<PostCallOutcome, BatchEnqueuedRefundError> {
        let canonical_id = canonical_provider_batch_id(batch_id);
        if let Some(pending) = pending {
            save::save_reservation(self, &canonical_id, pending).await?;
        }
        let lowered = status.to_lowercase();
        let mut refunded = false;
        if BATCH_ENQUEUED_REFUND_STATUSES.contains(&lowered.as_str())
            && let Some(reservation) = pop::pop_reservation(self, &canonical_id).await?
        {
            refund::refund(self, &reservation).await?;
            refunded = true;
        }
        Ok(PostCallOutcome {
            batch_id: canonical_id,
            saved: pending.is_some(),
            refunded,
        })
    }
}
