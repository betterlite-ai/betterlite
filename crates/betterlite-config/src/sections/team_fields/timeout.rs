use std::time::Duration;

use crate::loader::{ConfigError, EnvironmentLookup};

use super::{DEFAULT_TIMEOUT, RawScalar, ValidationTimeout, environment_seconds};

// A positive number is a deadline, positive infinity has none, and every other scalar keeps the default.
pub(super) fn resolve_timeout<L>(
    raw: Option<RawScalar>,
    lookup: &L,
) -> Result<ValidationTimeout, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(chosen) = raw else {
        return Ok(ValidationTimeout::Finite(DEFAULT_TIMEOUT));
    };
    let seconds = match chosen {
        RawScalar::Integer(seconds) => return Ok(integer_timeout(seconds)),
        RawScalar::Float(seconds) => seconds,
        RawScalar::Text(text) => match environment_seconds(&text, lookup)? {
            Some(seconds) => seconds,
            None => return Ok(ValidationTimeout::Finite(DEFAULT_TIMEOUT)),
        },
        RawScalar::Other(_) => return Ok(ValidationTimeout::Finite(DEFAULT_TIMEOUT)),
    };
    Ok(float_timeout(seconds))
}

fn integer_timeout(seconds: i64) -> ValidationTimeout {
    match u64::try_from(seconds) {
        Ok(0) | Err(_) => ValidationTimeout::Finite(DEFAULT_TIMEOUT),
        Ok(value) => ValidationTimeout::Finite(Duration::from_secs(value)),
    }
}

fn float_timeout(seconds: f64) -> ValidationTimeout {
    if seconds.is_nan() || seconds <= 0.0_f64 {
        return ValidationTimeout::Finite(DEFAULT_TIMEOUT);
    }
    if seconds.is_infinite() {
        return ValidationTimeout::Unlimited;
    }
    Duration::try_from_secs_f64(seconds)
        .map_or(ValidationTimeout::Unlimited, ValidationTimeout::Finite)
}
