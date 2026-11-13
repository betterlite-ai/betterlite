use super::*;
use crate::file_ids::managed_id::llm_output_file_id;
use rstest::rstest;

const NON_PREFIXED_OUTPUT_MARKER: &str = "aGVsbG87bGxtX291dHB1dF9maWxlX2lkLGZpbGUteA";

#[rstest]
#[case::unified_id_carries_an_output_file_id(FILE_UNIFIED, Some("file-xyz"))]
#[case::unified_id_without_an_output_field(FILE_UNIFIED_NO_OUTPUT, None)]
#[case::empty_output_file_id(FILE_UNIFIED_EMPTY_OUTPUT, Some(""))]
#[case::plain_id_has_no_output_file_id("file-plain", None)]
#[case::decoded_marker_behind_a_plain_prefix(NON_PREFIXED_OUTPUT_MARKER, None)]
fn model_file_ids_behavior_tests_behavior(#[case] file_id: &str, #[case] expected: Option<&str>) {
    assert_eq!(llm_output_file_id(file_id).as_deref(), expected);
}
