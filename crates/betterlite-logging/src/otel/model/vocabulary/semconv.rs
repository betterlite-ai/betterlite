use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OTELSemconvCategory {
    GenAiLatestExperimental,
}
impl OTELSemconvCategory {
    pub const GEN_AI_LATEST_EXPERIMENTAL: Self = Self::GenAiLatestExperimental;
}

#[must_use]
pub fn parse_semconv_opt_in(raw_value: Option<&str>) -> BTreeSet<OTELSemconvCategory> {
    let mut categories = BTreeSet::new();
    if let Some(raw) = raw_value {
        for token in raw.split(',').map(str::trim) {
            if token == "gen_ai_latest_experimental" {
                categories.insert(OTELSemconvCategory::GEN_AI_LATEST_EXPERIMENTAL);
            }
        }
    }
    categories
}

#[cfg(test)]
#[path = "semconv/tests.rs"]
mod tests;
