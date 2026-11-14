use std::sync::PoisonError;

use crate::api::batch_enqueued_reserve::keys::RESERVATION_RECORD_PREFIX;

use super::record::{ReservationRecord, record_ttl};
use super::store::{EnqueuedLocalStore, ReservationRecordScripts};
use super::{BatchEnqueuedRefundError, EnqueuedTokenRefund};

pub(crate) async fn save_reservation<Scripts, Store>(
    service: &EnqueuedTokenRefund<Scripts, Store>,
    batch_id: &str,
    reservation: &ReservationRecord,
) -> Result<(), BatchEnqueuedRefundError>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    let record_key = format!("{RESERVATION_RECORD_PREFIX}{batch_id}");
    let serialized =
        serde_json::to_string(reservation).map_err(|source| BatchEnqueuedRefundError::Encode {
            source: Box::new(source),
        })?;
    let elapsed = reservation
        .reserved_at_monotonic
        .mul_add(-1.0_f64, (service.clock)());
    let ttl_seconds = record_ttl(service.settings.ttl_seconds, elapsed);
    if let Some(scripts) = &service.scripts {
        if let Err(error) = scripts.save(&record_key, &serialized, ttl_seconds).await {
            tracing::warn!(
                error = %error,
                "Redis enqueued-token reservation save failed, falling back to in-memory"
            );
        } else {
            return Ok(());
        }
    }
    let mut store = service.store.lock().unwrap_or_else(PoisonError::into_inner);
    store.set_record(&record_key, &serialized, ttl_seconds)
}
