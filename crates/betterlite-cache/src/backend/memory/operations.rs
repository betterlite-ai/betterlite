use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use serde_json::Number;

use super::Ttl;
use crate::backend::BackendError;

pub(super) fn increment(
    values: &mut LocalTtlCache,
    key: &str,
    value: f64,
    ttl: Ttl,
) -> Result<f64, BackendError> {
    let base = match values.get(key)? {
        None => 0.0_f64,
        Some(CacheValue::Integer(number)) => {
            Number::from(number)
                .as_f64()
                .ok_or_else(|| BackendError::NotNumeric {
                    key: key.to_owned(),
                })?
        }
        Some(CacheValue::Float(number)) => number,
        Some(_) => {
            return Err(BackendError::NotNumeric {
                key: key.to_owned(),
            });
        }
    };
    let next = base.mul_add(1.0_f64, value);
    if !next.is_finite() {
        return Err(BackendError::IncrementOutOfRange {
            key: key.to_owned(),
        });
    }
    values.set(key, CacheValue::Float(next), ttl)?;
    Ok(next)
}
