use std::sync::PoisonError;

use crate::api::api_boundaries::{ApiBoundariesInput, JsonBoundary};
use crate::api::batch_enqueued_reserve::keys::RESERVATION_RECORD_PREFIX;

use super::record::ReservationRecord;
use super::store::{EnqueuedLocalStore, ReservationRecordScripts};
use super::{BatchEnqueuedRefundError, EnqueuedTokenRefund};

enum RedisRecord {
    Absent,
    Tombstone,
    Record(Vec<u8>),
}

pub(crate) async fn pop_reservation<Scripts, Store>(
    service: &EnqueuedTokenRefund<Scripts, Store>,
    batch_id: &str,
) -> Result<Option<ReservationRecord>, BatchEnqueuedRefundError>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    let record_key = format!("{RESERVATION_RECORD_PREFIX}{batch_id}");
    match pop_redis_record(service, &record_key).await {
        RedisRecord::Record(bytes) => return Ok(decode(service, batch_id, &bytes)),
        // the pop tombstones the record in place, so an empty hit means another poll already refunded it
        RedisRecord::Tombstone => {
            drop(pop_local_record(service, &record_key)?);
            return Ok(None);
        }
        RedisRecord::Absent => {}
    }
    let Some(bytes) = pop_local_record(service, &record_key)? else {
        return Ok(None);
    };
    Ok(decode(service, batch_id, &bytes))
}

async fn pop_redis_record<Scripts, Store>(
    service: &EnqueuedTokenRefund<Scripts, Store>,
    record_key: &str,
) -> RedisRecord
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    let Some(scripts) = &service.scripts else {
        return RedisRecord::Absent;
    };
    match scripts.pop(record_key, service.settings.ttl_seconds).await {
        Ok(Some(bytes)) if bytes.is_empty() => RedisRecord::Tombstone,
        Ok(Some(bytes)) => RedisRecord::Record(bytes),
        Ok(None) => RedisRecord::Absent,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Redis enqueued-token reservation pop failed, falling back to in-memory"
            );
            RedisRecord::Absent
        }
    }
}

fn pop_local_record<Scripts, Store>(
    service: &EnqueuedTokenRefund<Scripts, Store>,
    record_key: &str,
) -> Result<Option<Vec<u8>>, BatchEnqueuedRefundError>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    let mut store = service.store.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(record) = store.record(record_key)? else {
        return Ok(None);
    };
    store.delete(record_key);
    drop(store);
    Ok(Some(record))
}

fn decode<Scripts, Store>(
    service: &EnqueuedTokenRefund<Scripts, Store>,
    batch_id: &str,
    bytes: &[u8],
) -> Option<ReservationRecord>
where
    Scripts: ReservationRecordScripts,
    Store: EnqueuedLocalStore,
{
    let boundary = ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    });
    let decoded = if service.limits.check(boundary).is_ok() {
        serde_json::from_slice::<ReservationRecord>(bytes).ok()
    } else {
        None
    };
    let Some(record) = decoded else {
        tracing::warn!(
            batch_id = %batch_id,
            "Discarding malformed enqueued-token reservation record"
        );
        return None;
    };
    Some(record)
}
