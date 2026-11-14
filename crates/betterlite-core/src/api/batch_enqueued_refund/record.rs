use num_traits::cast::cast;
use serde::{Deserialize, Serialize};

use crate::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, ReservationBackend,
};

// the reservation as it crosses a store, named and defaulted as the source's serialized dataclass; an extra field from another version is ignored the way pydantic ignores it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReservationRecord {
    pub tokens: i64,
    pub scopes: Vec<BatchEnqueuedTokenScope>,
    #[serde(default = "default_backend")]
    pub backend: ReservationBackend,
    #[serde(default)]
    pub owner: String,
    pub reserved_at_monotonic: f64,
}

const fn default_backend() -> ReservationBackend {
    ReservationBackend::Redis
}

impl From<&BatchEnqueuedTokenReservation> for ReservationRecord {
    fn from(reservation: &BatchEnqueuedTokenReservation) -> Self {
        Self {
            tokens: reservation.tokens,
            scopes: reservation.scopes.clone(),
            backend: reservation.backend,
            owner: reservation.owner.clone(),
            reserved_at_monotonic: reservation.reserved_at_monotonic,
        }
    }
}

// the source keeps a record alive until the counters it refunds expire, shortened by the seconds it has already run
pub(crate) fn record_ttl(ttl_seconds: i64, elapsed_seconds: f64) -> i64 {
    let Some(elapsed) = cast::<f64, i64>(elapsed_seconds.ceil()) else {
        return 1;
    };
    ttl_seconds.saturating_sub(elapsed).max(1)
}
