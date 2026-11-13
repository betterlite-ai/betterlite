use std::num::ParseFloatError;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use thiserror::Error;

const MANTISSA_MASK: u64 = 0x000f_ffff_ffff_ffff;
const IMPLICIT_BIT: u64 = 0x0010_0000_0000_0000;
const MANTISSA_WIDTH: u32 = 52;
// the exponent field of a double plus the 1023 bias and the 52 mantissa bits
const EXPONENT_OFFSET: u64 = 1075;
const LARGEST_SHIFT: u32 = 100;

#[derive(Debug, Error)]
pub enum TimingError {
    #[error("the timestamp is not a number of seconds")]
    Seconds(#[source] ParseFloatError),
    #[error("the timestamp is outside the range the millisecond math covers")]
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Moment {
    pub seconds: i64,
    pub milliseconds: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    pub start_time: Moment,
    pub end_time: Moment,
}

// LiteLLM does this in doubles and .123 comes out as 122 ms, so the same double math is done in integers.
pub fn moment(time: DateTime<Utc>) -> Result<Moment, TimingError> {
    let micros = time.timestamp_micros();
    let elapsed = Duration::from_micros(micros.unsigned_abs());
    let text = format!("{}.{:06}", elapsed.as_secs(), elapsed.subsec_micros());
    let bits = text.parse::<f64>().map_err(TimingError::Seconds)?.to_bits();
    let (seconds, milliseconds) = if bits == 0 { (0, 0) } else { split(bits)? };
    let sign = if micros < 0 { -1_i64 } else { 1_i64 };
    Ok(Moment {
        seconds: seconds.checked_mul(sign).ok_or(TimingError::Range)?,
        milliseconds: milliseconds.checked_mul(sign).ok_or(TimingError::Range)?,
    })
}

// A double is significand * 2^-shift; the whole part is the seconds and the fraction is multiplied by 1000.
fn split(bits: u64) -> Result<(i64, i64), TimingError> {
    let significand = u128::from((bits & MANTISSA_MASK) | IMPLICIT_BIT);
    let shift = EXPONENT_OFFSET
        .checked_sub(bits.checked_shr(MANTISSA_WIDTH).ok_or(TimingError::Range)?)
        .and_then(|shift| u32::try_from(shift).ok())
        .filter(|shift| *shift <= LARGEST_SHIFT)
        .ok_or(TimingError::Range)?;
    let seconds = significand.checked_shr(shift).ok_or(TimingError::Range)?;
    let fraction_mask = 1_u128
        .checked_shl(shift)
        .and_then(|bit| bit.checked_sub(1))
        .ok_or(TimingError::Range)?;
    let product = (significand & fraction_mask)
        .checked_mul(1000)
        .ok_or(TimingError::Range)?;
    let milliseconds = round_to_double(product)
        .and_then(|rounded| rounded.checked_shr(shift))
        .ok_or(TimingError::Range)?;
    Ok((
        i64::try_from(seconds).map_err(|_source| TimingError::Range)?,
        i64::try_from(milliseconds).map_err(|_source| TimingError::Range)?,
    ))
}

// Rounds to 53 significant bits, ties to even, as a double multiplication does.
fn round_to_double(product: u128) -> Option<u128> {
    let dropped = product
        .checked_ilog2()
        .map_or(0, |top| top.saturating_sub(MANTISSA_WIDTH));
    if dropped == 0 {
        return Some(product);
    }
    let kept = product.checked_shr(dropped)?;
    let remainder = product.checked_sub(kept.checked_shl(dropped)?)?;
    let half = 1_u128.checked_shl(dropped.checked_sub(1)?)?;
    let round_up = remainder > half || (remainder == half && kept & 1 == 1);
    let rounded = if round_up { kept.checked_add(1)? } else { kept };
    rounded.checked_shl(dropped)
}

mod tests;
