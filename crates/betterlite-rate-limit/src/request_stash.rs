use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use tokio::sync::Mutex as AsyncMutex;

use crate::limiter_records::{
    ParallelSlotAcquisition, RateLimitResponse, RateLimitedModel, ReservationAwareIncrement,
    ReservationWindowIdentity,
};

mod tests;

#[derive(Clone, Debug, Default)]
pub struct RequestStashSlot {
    stash: Arc<Mutex<Option<Arc<RequestStash>>>>,
}

#[derive(Debug)]
pub struct RequestStash {
    pub state: Mutex<StashState>,
    pub parallel_slot_release_lock: AsyncMutex<()>,
}

#[derive(Debug, Default)]
pub struct StashState {
    pub owner_litellm_call_id: Option<String>,
    pub rate_limit_response: Option<RateLimitResponse>,
    pub parallel_slot: Option<ParallelSlotAcquisition>,
    pub reserved_tokens: i64,
    pub reserved_model: Option<RateLimitedModel>,
    pub reserved_scopes: BTreeSet<(String, String)>,
    pub itpm_reserved_tokens: i64,
    pub itpm_reserved_scopes: BTreeSet<(String, String)>,
    pub itpm_reserved_windows: BTreeSet<ReservationWindowIdentity>,
    pub otpm_reserved_tokens: i64,
    pub otpm_reserved_scopes: BTreeSet<(String, String)>,
    pub otpm_reserved_windows: BTreeSet<ReservationWindowIdentity>,
    pub batch_enqueued_reservation: Option<BatchEnqueuedTokenReservation>,
    pub batch_tpd_refund_ops: Vec<ReservationAwareIncrement>,
    pub reservation_released: bool,
    pub tpm_limited_tags: BTreeSet<String>,
}

impl RequestStashSlot {
    #[must_use]
    pub fn get(&self) -> Option<Arc<RequestStash>> {
        self.stash
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn get_or_create(&self) -> Arc<RequestStash> {
        let mut slot = self.stash.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = slot.as_ref() {
            return Arc::clone(existing);
        }
        let created = Arc::new(RequestStash {
            state: Mutex::new(StashState::default()),
            parallel_slot_release_lock: AsyncMutex::new(()),
        });
        *slot = Some(Arc::clone(&created));
        created
    }

    // a call id that is not a string leaves the owner alone, so a second claim never clears the first
    #[must_use]
    pub fn claim_for_call_id(&self, call_id: Option<&str>) -> Arc<RequestStash> {
        let stash = self.get_or_create();
        if let Some(given) = call_id {
            let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.owner_litellm_call_id = Some(given.to_owned());
        }
        stash
    }

    #[must_use]
    pub fn for_call(&self, call_id: Option<&str>) -> Option<Arc<RequestStash>> {
        let stash = self.get()?;
        let owner = stash
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .owner_litellm_call_id
            .clone();
        match (&owner, call_id) {
            (None, _) | (_, None) => Some(stash),
            (Some(owner_id), Some(given)) if owner_id.as_str() == given => Some(stash),
            _ => None,
        }
    }
}
