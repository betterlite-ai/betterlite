use std::collections::BTreeSet;

use betterlite_config::loader::EnvironmentLookup;

use crate::otel::config::destination::OtelDestination;

pub const ADDITIVE_DESTINATION_MODE: &str = "additive";
pub const TENANT_DESTINATION_MODE_ENV: &str = "LITELLM_OTEL_TENANT_DESTINATION_MODE";

// litellm.integrations.otel.plumbing.context.tenant_destinations_are_additive
#[must_use]
pub fn destinations_are_additive(
    configured: Option<&str>,
    environment: &dyn EnvironmentLookup,
) -> bool {
    configured.map_or_else(
        || {
            environment
                .lookup(TENANT_DESTINATION_MODE_ENV)
                .is_some_and(|value| value.trim().eq_ignore_ascii_case(ADDITIVE_DESTINATION_MODE))
        },
        |value| value.trim().eq_ignore_ascii_case(ADDITIVE_DESTINATION_MODE),
    )
}

// litellm.integrations.otel.plumbing.context.destination_backends
#[must_use]
pub fn destination_backends(destinations: &[OtelDestination]) -> BTreeSet<String> {
    destinations
        .iter()
        .filter_map(|destination| destination.as_ref().callback_name.clone())
        .collect()
}

// litellm.integrations.otel.plumbing.context.suppressed_backends; override is the default mode
#[must_use]
pub fn suppressed_backends(destinations: &[OtelDestination], additive: bool) -> BTreeSet<String> {
    if additive {
        return BTreeSet::new();
    }
    destination_backends(destinations)
}

#[path = "destination_routing/tests.rs"]
mod tests;
