use super::*;
use crate::file_ids::managed_id::is_unified_vector_id;
use rstest::rstest;

#[rstest]
#[case::encoded_unified_id(VECTOR_UNIFIED, true)]
#[case::encoded_unified_id_without_a_resource(VECTOR_UNIFIED_NO_RESOURCE, true)]
#[case::plain_id("vs_plain", false)]
#[case::plain_word("not-an-id", false)]
#[case::raw_unified_id("litellm_proxy:vector_store;unified_id,u", false)]
fn model_file_ids_behavior_tests_behavior(#[case] vector_id: &str, #[case] expected: bool) {
    assert_eq!(is_unified_vector_id(vector_id), expected);
}
