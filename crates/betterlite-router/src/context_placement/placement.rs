use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::context_placement as section;

use crate::complexity_math::window_holds;
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::tier_definitions::model::TierSet;

use super::error::PlacementError;
use super::facts::WindowFacts;
use super::fit::window_facts;
use super::request::RequestFacts;

// _ContextWindowPlacement: the placement tier, the subset of its pool the pick may use, every holdable group
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub tier: Tier,
    pub allowed_models: Vec<String>,
    pub holdable_models: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedPlacement {
    pub tier: Tier,
    pub signals: Vec<String>,
    pub original_tier: Option<Tier>,
}

pub struct PlacementRequest<'request> {
    pub tier: &'request Tier,
    pub set: &'request TierSet,
    pub default_model: Option<&'request str>,
    pub pool_override: Option<&'request [String]>,
    pub request: RequestFacts<'request>,
}

pub(super) struct TokenInput<'request> {
    pub(super) set: &'request TierSet,
    pub(super) tier: &'request Tier,
    pub(super) pool: &'request [String],
    pub(super) facts: &'request BTreeMap<String, WindowFacts>,
    pub(super) needed: u64,
}

// _apply_context_placement: an escalation adds the signal and names the origin tier; an unchanged tier adds none
#[must_use]
pub fn apply_context_placement(
    tier: Tier,
    signals: Vec<String>,
    placement: Option<&Placement>,
) -> AppliedPlacement {
    let Some(placement) = placement else {
        return AppliedPlacement {
            tier,
            signals,
            original_tier: None,
        };
    };
    if placement.tier.name() == tier.name() {
        return AppliedPlacement {
            tier: placement.tier.clone(),
            signals,
            original_tier: None,
        };
    }
    let mut escalated = signals;
    escalated.push("context_escalation".to_owned());
    AppliedPlacement {
        tier: placement.tier.clone(),
        signals: escalated,
        original_tier: Some(tier),
    }
}

// _placement_for_tokens: keep the tier when some of its pool holds, else walk up to the first provable group
pub(super) fn placement_for_tokens(
    config: &section::Config,
    input: &TokenInput<'_>,
) -> Result<Option<Placement>, PlacementError> {
    let buffer = config.buffer;
    let in_tier = holding_in_tier(input, buffer);
    if !in_tier.is_empty() && in_tier.len() == input.pool.len() {
        return Ok(None);
    }
    let holdable = holdable_models(input, buffer);
    if !in_tier.is_empty() {
        return Ok(Some(Placement {
            tier: input.tier.clone(),
            allowed_models: in_tier,
            holdable_models: holdable,
        }));
    }
    let names = input.set.tier_names();
    let Some(position) = names.iter().position(|name| *name == input.tier.name()) else {
        return Err(PlacementError::UnknownTier {
            tier: input.tier.name().to_owned(),
        });
    };
    for name in names.iter().skip(position.saturating_add(1)) {
        let proven = proven_models(input, *name, buffer);
        if !proven.is_empty() {
            let tier = match ComplexityTier::built_in(*name) {
                Some(built_in) if input.set.definitions.is_none() => Tier::BuiltIn(built_in),
                _ => Tier::Named((*name).to_owned()),
            };
            return Ok(Some(Placement {
                tier,
                allowed_models: proven,
                holdable_models: holdable,
            }));
        }
    }
    Ok(None)
}

// the pool groups whose window holds the request
fn holding_in_tier(input: &TokenInput<'_>, buffer: f64) -> Vec<String> {
    let mut in_tier = Vec::new();
    for group in input.pool {
        let window = window_facts(input.facts, group).window;
        if window_holds(window, Some(input.needed), buffer) {
            in_tier.push(group.clone());
        }
    }
    in_tier
}

// every group in every tier pool whose window holds the request
fn holdable_models(input: &TokenInput<'_>, buffer: f64) -> BTreeSet<String> {
    let mut holdable = BTreeSet::new();
    for pool in input.set.pools.values() {
        for entry in pool.models() {
            let window = window_facts(input.facts, &entry.model_name).window;
            if window_holds(window, Some(input.needed), buffer) {
                holdable.insert(entry.model_name.clone());
            }
        }
    }
    holdable
}

// the pool groups that provably fit the request, the only ones an escalation may land on
fn proven_models(input: &TokenInput<'_>, name: &str, buffer: f64) -> Vec<String> {
    let mut proven = Vec::new();
    if let Some(pool) = input.set.pools.get(name) {
        for entry in pool.models() {
            if window_facts(input.facts, &entry.model_name).provably_fits(input.needed, buffer) {
                proven.push(entry.model_name.clone());
            }
        }
    }
    proven
}
