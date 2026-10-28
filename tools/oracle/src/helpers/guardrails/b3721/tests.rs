#![cfg(test)]

use crate::case::HelperCase;
use crate::helpers::{HelperRun, run_case};
use std::{fs, path::Path};

#[rstest::rstest]
#[case::contextual_grounding_fallback("contextual-grounding-fallback")]
#[case::count_list_strings_skips("count-list-strings-skips")]
#[case::count_object_blocks("count-object-blocks")]
#[case::empty_mask_keeps_originals("empty-mask-keeps-originals")]
#[case::empty_output_list("empty-output-list")]
#[case::empty_structured_fallback("empty-structured-fallback")]
#[case::empty_target_indices("empty-target-indices")]
#[case::explicit_prevents_fallback("explicit-prevents-fallback")]
#[case::fallback_disabled("fallback-disabled")]
#[case::fallback_needs_query("fallback-needs-query")]
#[case::fallback_needs_source("fallback-needs-source")]
#[case::filtered_messages_merge_back_by_index("filtered-messages-merge-back-by-index")]
#[case::grounding_role_boundary("grounding-role-boundary")]
#[case::grounding_source_roles_trusted("grounding-source-roles-trusted")]
#[case::input_absent("input-absent")]
#[case::input_unqualified_output_grounded("input-unqualified-output-grounded")]
#[case::latest_user_selection("latest-user-selection")]
#[case::legacy_latest_text("legacy-latest-text")]
#[case::masked_text_extraction("masked-text-extraction")]
#[case::message_masking_index_tracking("message-masking-index-tracking")]
#[case::message_null_and_metadata("message-null-and-metadata")]
#[case::no_user_skips("no-user-skips")]
#[case::null_output_list("null-output-list")]
#[case::output_grounded("output-grounded")]
#[case::output_plain("output-plain")]
#[case::output_without_response("output-without-response")]
#[case::prepare_absent("prepare-absent")]
#[case::prepare_latest("prepare-latest")]
#[case::prepare_unfiltered("prepare-unfiltered")]
#[case::response_choice_masking("response-choice-masking")]
#[case::response_empty_content("response-empty-content")]
#[case::response_legacy_selection("response-legacy-selection")]
#[case::role_subset_mismatch_guard("role-subset-mismatch-guard")]
#[case::scanned_slice_alignment("scanned-slice-alignment")]
#[case::structured_precedence("structured-precedence")]
#[case::whole_list_masked_output("whole-list-masked-output")]
fn cases_match_litellm(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/bedrock-messages")
        .join(format!("{name}.yaml"));
    let source = fs::read_to_string(path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    let result = run_case(&case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
