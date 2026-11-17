use fancy_regex::{Regex, RegexBuilder};

use crate::tag_matchers::patterns::MAX_MATCH_STEPS;

pub const MAX_RULES_BYTES: usize = 0x0010_0000;

// the case-insensitive search B2229 compiles, rebuilt with the step limit tag matching already uses
pub(crate) fn bounded(source: &str) -> Option<Regex> {
    RegexBuilder::new(source)
        .case_insensitive(true)
        .backtrack_limit(MAX_MATCH_STEPS)
        .seek(false)
        .build()
        .ok()
}
