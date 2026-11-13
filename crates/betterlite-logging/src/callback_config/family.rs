use std::collections::{BTreeMap, BTreeSet};

use strum::Display;

use super::CallbackVars;
use super::error::ValidationError;

pub const SPAN_SCOPE_VAR: &str = "langfuse_span_scope";
pub const SAMPLING_RATE_VARS: [&str; 2] =
    ["arize_error_sampling_rate", "arize_success_sampling_rate"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display)]
pub enum Family {
    Arize,
    Datadog,
    #[strum(serialize = "GCS")]
    Gcs,
    Humanloop,
    Langfuse,
    LangSmith,
    #[strum(serialize = "New Relic")]
    NewRelic,
    PostHog,
    #[strum(serialize = "Weights & Biases")]
    WeightsAndBiases,
}

const PREFIXES: [(&str, Family); 10] = [
    ("arize_", Family::Arize),
    ("dd_", Family::Datadog),
    ("gcs_", Family::Gcs),
    ("humanloop_", Family::Humanloop),
    ("langfuse_", Family::Langfuse),
    ("langsmith_", Family::LangSmith),
    ("newrelic_", Family::NewRelic),
    ("posthog_", Family::PostHog),
    ("wandb_", Family::WeightsAndBiases),
    ("weave_", Family::WeightsAndBiases),
];

// Options pick what a family exports, not where it exports to, so they carry no credentials.
#[must_use]
pub fn family_of(var: &str) -> Option<Family> {
    if var == SPAN_SCOPE_VAR || SAMPLING_RATE_VARS.contains(&var) {
        return None;
    }
    PREFIXES
        .iter()
        .find(|(prefix, _)| var.starts_with(prefix))
        .map(|(_, family)| *family)
}

pub fn cross_entry_family_error(
    callback_vars: &CallbackVars,
    stored_vars_by_entry: &[CallbackVars],
) -> Result<(), ValidationError> {
    let mut stored_by_var = BTreeMap::new();
    let mut family_values = BTreeSet::new();
    for (var, value) in stored_vars_by_entry.iter().flatten() {
        if let Some(family) = family_of(var) {
            stored_by_var.insert(var.as_str(), value.as_str());
            family_values.insert((family, value.as_str()));
        }
    }
    let held_families: BTreeSet<Family> = family_values.iter().map(|(family, _)| *family).collect();
    for (var, value) in callback_vars {
        let Some(family) = family_of(var).filter(|family| held_families.contains(family)) else {
            continue;
        };
        let changed = stored_by_var.get(var.as_str()).map_or_else(
            || !family_values.contains(&(family, value.as_str())),
            |stored| stored != value,
        );
        if changed {
            return Err(ValidationError::FamilyConflict {
                family,
                var: var.clone(),
            });
        }
    }
    Ok(())
}

pub fn conflicting_span_scope_error(
    callback_vars: &CallbackVars,
    stored_vars_by_entry: &[CallbackVars],
) -> Result<(), ValidationError> {
    let Some(incoming) = callback_vars.get(SPAN_SCOPE_VAR) else {
        return Ok(());
    };
    for stored in stored_vars_by_entry
        .iter()
        .filter_map(|entry| entry.get(SPAN_SCOPE_VAR))
    {
        if stored != incoming {
            return Err(ValidationError::SpanScopeConflict {
                stored: stored.clone(),
            });
        }
    }
    Ok(())
}
