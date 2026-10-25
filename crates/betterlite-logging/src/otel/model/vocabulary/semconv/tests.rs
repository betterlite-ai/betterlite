#![cfg(test)]

use crate::otel::vocabulary::*;

#[test]
fn semconv_opt_in_parses_known_tokens() {
    let categories = parse_semconv_opt_in(Some(" unknown, gen_ai_latest_experimental, "));
    assert_eq!(categories.len(), 1);
    assert!(categories.contains(&OTELSemconvCategory::GEN_AI_LATEST_EXPERIMENTAL));
    assert!(parse_semconv_opt_in(None).is_empty());
}
