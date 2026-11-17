use std::collections::BTreeSet;

use tracing::warn;

use super::attributes::METRIC_ATTRIBUTE_CEILING;
use crate::otel::legacy_metrics::filter::{ResolvedFilter, TOKEN_TYPE_ATTRIBUTE};

// the shared validator accepts every span name, so a metric-ineligible one would be a silent no-op
pub(super) fn warn_about_ineligible_names(filter: &ResolvedFilter) {
    let named: BTreeSet<&str> = filter
        .include
        .iter()
        .chain(&filter.exclude)
        .flatten()
        .map(String::as_str)
        .collect();
    let ineligible: Vec<&str> = named
        .into_iter()
        .filter(|name| *name != TOKEN_TYPE_ATTRIBUTE && !METRIC_ATTRIBUTE_CEILING.contains(name))
        .collect();
    if ineligible.is_empty() {
        return;
    }
    let message = format!(
        "OTel metrics: {} cannot be a metric attribute and is being ignored; it varies per request or is client-supplied, so it would make one time series per request. It is still on the span. Metric attributes are limited to: {}",
        ineligible.join(", "),
        METRIC_ATTRIBUTE_CEILING.join(", ")
    );
    warn!("{message}");
}
