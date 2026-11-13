use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;

use super::error::ReplicateError;

const MILLISECONDS_PER_SECOND: Decimal = Decimal::ONE_THOUSAND;
const NANOSECONDS_PER_SECOND: u32 = 1_000_000_000;

// The stamps a Replicate response carries; an absent one is replaced by the clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReplicateStamps {
    pub created: Option<Decimal>,
    pub ended: Option<Decimal>,
}

// Ports get_replicate_completion_pricing: a zero total_time is replaced by ended - created, and the product is still divided by 1000.
pub fn replicate_charge(
    price_per_second: Decimal,
    total_time: Decimal,
    created: Decimal,
    ended: Decimal,
) -> Result<Decimal, ReplicateError> {
    let elapsed = if total_time.is_zero() {
        ended.checked_sub(created).ok_or(ReplicateError::Overflow)?
    } else {
        total_time
    };
    price_per_second
        .checked_mul(elapsed)
        .and_then(|product| product.checked_div(MILLISECONDS_PER_SECOND))
        .ok_or(ReplicateError::Overflow)
}

fn epoch_seconds(moment: SystemTime) -> Result<Decimal, ReplicateError> {
    let since_epoch = moment
        .duration_since(UNIX_EPOCH)
        .map_err(|_source| ReplicateError::ClockBeforeEpoch)?;
    Decimal::from(since_epoch.subsec_nanos())
        .checked_div(Decimal::from(NANOSECONDS_PER_SECOND))
        .and_then(|fraction| fraction.checked_add(Decimal::from(since_epoch.as_secs())))
        .ok_or(ReplicateError::Overflow)
}

// Both Python defaults are evaluated before either is used, so a zero total_time reads the clock twice, created first.
pub fn replicate_pricing<Clock>(
    price_per_second: Decimal,
    total_time: Decimal,
    stamps: ReplicateStamps,
    clock: &mut Clock,
) -> Result<Decimal, ReplicateError>
where
    Clock: FnMut() -> SystemTime,
{
    if !total_time.is_zero() {
        return replicate_charge(price_per_second, total_time, Decimal::ZERO, Decimal::ZERO);
    }
    let created_default = epoch_seconds(clock())?;
    let ended_default = epoch_seconds(clock())?;
    replicate_charge(
        price_per_second,
        total_time,
        stamps.created.unwrap_or(created_default),
        stamps.ended.unwrap_or(ended_default),
    )
}
