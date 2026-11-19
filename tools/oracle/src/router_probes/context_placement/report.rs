use std::collections::BTreeMap;

use betterlite_router::context_placement::ContextPlacement;
use betterlite_router::context_placement::fit::ContextFit;
use betterlite_router::context_placement::placement::{AppliedPlacement, Placement};

use super::wire::{AppliedOutput, GroupInput, PlacementOutput, WindowFact};

#[must_use]
pub(super) fn facts(fit: &ContextFit) -> BTreeMap<String, WindowFact> {
    fit.facts
        .iter()
        .map(|(name, facts)| {
            (
                name.clone(),
                WindowFact {
                    window: facts.window,
                    has_unknown: facts.has_unknown,
                },
            )
        })
        .collect()
}

#[must_use]
pub(super) fn ceilings(
    service: &ContextPlacement<'_>,
    groups: &[GroupInput],
) -> BTreeMap<String, Option<u64>> {
    groups
        .iter()
        .map(|group| {
            (
                group.model.clone(),
                service.group_output_ceiling(&group.model),
            )
        })
        .collect()
}

#[must_use]
pub(super) fn placement_output(placement: Placement) -> PlacementOutput {
    PlacementOutput {
        tier: placement.tier.name().to_owned(),
        allowed_models: placement.allowed_models,
        holdable_models: placement.holdable_models.into_iter().collect(),
    }
}

#[must_use]
pub(super) fn applied_output(applied: AppliedPlacement) -> AppliedOutput {
    AppliedOutput {
        tier: applied.tier.name().to_owned(),
        signals: applied.signals,
        original_tier: applied
            .original_tier
            .as_ref()
            .map(|tier| tier.name().to_owned()),
    }
}
