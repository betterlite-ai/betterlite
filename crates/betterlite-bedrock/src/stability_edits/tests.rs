#![cfg(test)]

mod cases;
mod corpus;
mod support;

use rstest::rstest;

#[rstest]
#[case::stability_edit_model_selection(cases::stability_edit_model_selection)]
#[case::image_and_mask_fields(cases::image_and_mask_fields)]
#[case::native_edit_options(cases::native_edit_options)]
#[case::stability_artifact_results(cases::stability_artifact_results)]
#[case::filtered_result_error(cases::filtered_result_error)]
#[case::stability_edit_url(cases::stability_edit_url)]
#[case::stability_option_mapping(cases::stability_option_mapping)]
fn stability_edits_cases(#[case] case: fn()) {
    case();
}
