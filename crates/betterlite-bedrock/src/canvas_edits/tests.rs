#![cfg(test)]

mod cases;
mod contract;
mod options;
mod support;

use rstest::rstest;

#[rstest]
#[case::case_1_canvas_task_types(cases::canvas_task_types)]
#[case::case_2_canvas_catalog_eligibility(options::canvas_catalog_eligibility)]
#[case::case_3_image_bytes_base64(cases::image_bytes_base64)]
#[case::case_4_tuple_file_rejection(cases::tuple_file_rejection)]
#[case::case_5_size_parse_fallback(options::size_parse_fallback)]
#[case::case_6_quality_map(options::quality_map)]
#[case::case_7_response_format_warning(options::response_format_warning)]
#[case::case_8_mask_versus_mask_prompt(cases::mask_versus_mask_prompt)]
#[case::case_9_canvas_task_errors(cases::canvas_task_errors)]
#[case::case_10_canvas_edit_contract(contract::canvas_edit_contract)]
#[case::case_11_canvas_edit_selector(contract::canvas_edit_selector)]
fn canvas_edits_cases(#[case] case: fn()) {
    case();
}
