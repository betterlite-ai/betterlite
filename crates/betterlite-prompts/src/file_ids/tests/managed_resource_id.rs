use super::*;
use crate::file_ids::managed_id::provider_resource_id;
use rstest::rstest;

const NON_PREFIXED_RESOURCE: &str = "aGVsbG87cmVzb3VyY2VfaWQsdnNfaGlkZGVu";
const VECTOR_BY_VECTOR_ID: &str =
    "bGl0ZWxsbV9wcm94eTp2ZWN0b3Jfc3RvcmU7dW5pZmllZF9pZCx1O3ZlY3Rvcl9zdG9yZV9pZCx2c192";
const VECTOR_BY_FILE_ID: &str =
    "bGl0ZWxsbV9wcm94eTp2ZWN0b3Jfc3RvcmU7dW5pZmllZF9pZCx1O2ZpbGVfaWQsZmlsZS1m";

#[rstest]
#[case::resource_id_field(VECTOR_UNIFIED, Some("vs_abc123"))]
#[case::vector_store_id_field(VECTOR_BY_VECTOR_ID, Some("vs_v"))]
#[case::file_id_field(VECTOR_BY_FILE_ID, Some("file-f"))]
#[case::raw_unified_id(
    "litellm_proxy:vector_store;unified_id,u;resource_id,vs_raw",
    Some("vs_raw")
)]
#[case::plain_id("vs_plain", None)]
#[case::decoded_marker_behind_a_plain_prefix(NON_PREFIXED_RESOURCE, None)]
fn model_file_ids_behavior_tests_behavior(
    #[case] unified_id: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(provider_resource_id(unified_id).as_deref(), expected);
}
