use std::collections::BTreeSet;
use std::str::FromStr as _;

use crate::otel::vocabulary::span_names::is_python_space;

#[path = "semconv/tests.rs"]
mod tests;

pub const STABILITY_OPT_IN_ENV: &str = "OTEL_SEMCONV_STABILITY_OPT_IN";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum OTELSemconvCategory {
    GenAiLatestExperimental,
}

#[must_use]
pub fn parse_semconv_opt_in(raw: &str) -> BTreeSet<OTELSemconvCategory> {
    let mut categories = BTreeSet::new();
    // the convention ignores opt-in tokens it does not know
    for token in raw.split(',') {
        if let Ok(category) = OTELSemconvCategory::from_str(token.trim_matches(is_python_space)) {
            categories.insert(category);
        }
    }
    categories
}
