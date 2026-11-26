#![cfg(test)]

mod cases;
mod contract;
mod options;
mod support;

use rstest::rstest;

#[rstest]
#[case::canvas_task_types(cases::canvas_task_types)]
#[case::canvas_catalog_eligibility(options::canvas_catalog_eligibility)]
#[case::image_bytes_base64(cases::image_bytes_base64)]
#[case::tuple_file_rejection(cases::tuple_file_rejection)]
#[case::size_parse_fallback(options::size_parse_fallback)]
#[case::quality_map(options::quality_map)]
#[case::response_format_warning(options::response_format_warning)]
#[case::mask_versus_mask_prompt(cases::mask_versus_mask_prompt)]
#[case::canvas_task_errors(cases::canvas_task_errors)]
#[case::canvas_edit_contract(contract::canvas_edit_contract)]
#[case::canvas_edit_selector(contract::canvas_edit_selector)]
fn canvas_edits_cases(#[case] case: fn()) {
    case();
}
