use super::*;
use crate::file_ids::managed_id::original_file_id;
use rstest::rstest;

const INNER_SEMICOLON_EMBEDDED: &str = "file-bGl0ZWxsbTphO2I7bW9kZWwsYw";
const EMPTY_MODEL_EMBEDDED: &str = "file-bGl0ZWxsbTpmaWxlLXo7bW9kZWwsO3gseQ";
const BLANK_MODEL_EMBEDDED: &str = "file-bGl0ZWxsbTpmaWxlLXc7bW9kZWwsIDt4";

#[rstest]
#[case::model_embedded_id(FILE_EMBEDDED, Some("file-abc123"))]
#[case::blank_model_still_counts_as_embedded(BLANK_MODEL_EMBEDDED, Some("file-w"))]
#[case::field_without_a_model_value(EMPTY_MODEL_EMBEDDED, None)]
#[case::separator_before_the_model_field(INNER_SEMICOLON_EMBEDDED, Some(INNER_SEMICOLON_EMBEDDED))]
#[case::plain_id("file-plain", None)]
#[case::unified_id(FILE_UNIFIED, None)]
fn model_file_ids_behavior_tests_behavior(#[case] file_id: &str, #[case] expected: Option<&str>) {
    assert_eq!(original_file_id(file_id).as_deref(), expected);
}
