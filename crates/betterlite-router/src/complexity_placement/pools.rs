use std::collections::BTreeMap;

use rand::{Rng, RngExt as _};
use rust_decimal::prelude::ToPrimitive as _;

use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::tier_definitions::model::{TierModelEntry, TierPool, TierSet};

use super::TierPlacement;
use super::error::TierPlacementError;

const MEDIUM: ComplexityTier = ComplexityTier::Medium;

// _tier_pools: every configured tier as its model names, a single value as a one-model list.
#[must_use]
pub fn tier_pools(set: &TierSet) -> BTreeMap<String, Vec<String>> {
    set.pools
        .iter()
        .map(|(tier, pool)| (tier.clone(), model_names(pool)))
        .collect()
}

#[must_use]
pub fn model_names(pool: &TierPool) -> Vec<String> {
    pool.models()
        .iter()
        .map(|entry| entry.model_name.clone())
        .collect()
}

// get_model_for_tier: the tier's own pool, then the configured default model, then MEDIUM.
pub fn model_for_tier<R>(
    placement: &TierPlacement<'_>,
    tier: &Tier,
    random: &mut R,
) -> Result<String, TierPlacementError>
where
    R: Rng + ?Sized,
{
    let tier_key = tier.name();
    if let Some(pool) = placement.tiers.pools.get(tier_key) {
        return pick_from_pool(pool, tier_key, random);
    }
    // an explicit value is used as given, an empty one too; only an absent default falls through
    if let Some(default_model) = placement.default_model {
        return Ok(default_model.to_owned());
    }
    if let Some(pool) = placement.tiers.pools.get(MEDIUM.name()) {
        return pick_from_pool(pool, MEDIUM.name(), random);
    }
    Err(TierPlacementError::NoModelForTier {
        tier: tier_key.to_owned(),
    })
}

// _pick_from_tier_value: a bare name is returned without a draw, and an empty pool raises.
pub fn pick_from_pool<R>(
    pool: &TierPool,
    tier_key: &str,
    random: &mut R,
) -> Result<String, TierPlacementError>
where
    R: Rng + ?Sized,
{
    match pool {
        TierPool::Single(entry) => Ok(entry.model_name.clone()),
        TierPool::Pool(entries) if entries.is_empty() => Err(TierPlacementError::EmptyPool {
            tier: tier_key.to_owned(),
        }),
        TierPool::Pool(entries) => {
            let picked = draw_index(entries.len(), random)?;
            entry_model(entries, picked)
        }
    }
}

// a caller-supplied pool always reaches random.choice, even for one name
pub fn pick_from_names<R>(
    names: &[String],
    tier_key: &str,
    random: &mut R,
) -> Result<String, TierPlacementError>
where
    R: Rng + ?Sized,
{
    if names.is_empty() {
        return Err(TierPlacementError::EmptyPool {
            tier: tier_key.to_owned(),
        });
    }
    let picked = draw_index(names.len(), random)?;
    let Some(name) = names.get(picked) else {
        return Err(TierPlacementError::Draw);
    };
    Ok(name.clone())
}

// Python's int(random() * len(seq)): one 53-bit draw, truncated; a draw past the end fails in the caller's lookup.
pub(crate) fn draw_index<R>(count: usize, random: &mut R) -> Result<usize, TierPlacementError>
where
    R: Rng + ?Sized,
{
    let Some(width) = count.to_f64() else {
        return Err(TierPlacementError::Draw);
    };
    // float arithmetic operators are denied, so the product is one fused multiply-add
    let scaled = random.random::<f64>().mul_add(width, 0.0);
    usize::try_from(scaled.to_i64().unwrap_or(i64::MAX)).map_err(|_error| TierPlacementError::Draw)
}

pub(crate) fn entry_model(
    entries: &[TierModelEntry],
    picked: usize,
) -> Result<String, TierPlacementError> {
    let Some(entry) = entries.get(picked) else {
        return Err(TierPlacementError::Draw);
    };
    Ok(entry.model_name.clone())
}
