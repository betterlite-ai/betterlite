use rstest::rstest;

use super::{auth, flux2, inline, mai, reply, selection, url};

#[rstest]
#[case::flux_json_images(flux2::flux_json_images)]
#[case::flux_inline_base64(inline::flux_inline_base64)]
#[case::mai_edit_prompt_images(mai::mai_edit_prompt_images)]
#[case::mai_edit_usage(reply::mai_edit_usage)]
#[case::image_edit_api_version(url::image_edit_api_version)]
#[case::image_edit_model_selection(selection::image_edit_model_selection)]
#[case::flux1_missing_version(url::flux1_missing_version)]
#[case::flux2_preview_default(url::flux2_preview_default)]
#[case::mai_preview_ignores_global(url::mai_preview_ignores_global)]
#[case::foundry_edit_auth(auth::foundry_edit_auth)]
fn image_edits_cases(#[case] check: fn()) {
    check();
}
