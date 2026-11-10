use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::logging_pre_post::masking::masked_headers;

fn chain(levels: usize, leaf: &str) -> String {
    format!(
        "{}\"{leaf}\"{}",
        "{\"key\":".repeat(levels.saturating_add(1)),
        "}".repeat(levels.saturating_add(1))
    )
}

fn parsed(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the fixture is valid")
}

#[rstest]
#[case::last_masked_level(20, "ab****gh")]
#[case::first_unmasked_level(21, "abcdefgh")]
fn logging_pre_post_behavior_tests_behavior(#[case] levels: usize, #[case] shown_leaf: &str) {
    assert_eq!(
        masked_headers(&parsed(&chain(levels, "abcdefgh")), false),
        parsed(&chain(levels, shown_leaf))
    );
}
