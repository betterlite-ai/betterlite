use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RolloverPolicy {
    pub enabled: bool,
}

// LiteLLM enables rollover only when litellm.budget_rollover is exactly True; the phase owner builds this value from the SDK flag.
#[must_use]
pub fn reset_spend(
    policy: RolloverPolicy,
    spend: Option<Decimal>,
    max_budget: Option<Decimal>,
) -> Decimal {
    if !policy.enabled {
        return Decimal::ZERO;
    }
    carried_spend(spend, max_budget)
}

// max(0, spend - cap): a missing cap has nothing to carry against, and a zero cap carries the whole spend forward.
#[must_use]
pub fn carried_spend(spend: Option<Decimal>, cap: Option<Decimal>) -> Decimal {
    let Some(limit) = cap else {
        return Decimal::ZERO;
    };
    let amount = spend.unwrap_or_default();
    amount
        .checked_sub(limit)
        .unwrap_or_default()
        .max(Decimal::ZERO)
}

// _row_carried_spend: a row with no linked budget, or one whose tier is not in the caps map, carries nothing.
#[must_use]
pub fn row_carried_spend(
    spend: Option<Decimal>,
    budget_id: Option<&str>,
    caps: &BTreeMap<String, Decimal>,
) -> Decimal {
    carried_spend(
        spend,
        budget_id.and_then(|identifier| caps.get(identifier).copied()),
    )
}

mod tests;
