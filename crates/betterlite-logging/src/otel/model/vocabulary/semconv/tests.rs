#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use crate::otel::vocabulary::semconv::{OTELSemconvCategory, parse_semconv_opt_in};

#[rstest]
#[case::empty("", BTreeSet::new())]
#[case::only_separators(" , ,", BTreeSet::new())]
#[case::unknown_tokens_ignored("http, unknown", BTreeSet::new())]
#[case::case_sensitive("GEN_AI_LATEST_EXPERIMENTAL", BTreeSet::new())]
#[case::padded_with_trailing_comma(
    " http , gen_ai_latest_experimental ,",
    BTreeSet::from([OTELSemconvCategory::GenAiLatestExperimental])
)]
fn opt_in_keeps_only_known_categories(
    #[case] raw: &str,
    #[case] expected: BTreeSet<OTELSemconvCategory>,
) {
    assert_eq!(parse_semconv_opt_in(raw), expected);
}
