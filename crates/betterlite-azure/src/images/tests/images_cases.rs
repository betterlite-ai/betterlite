#![cfg(test)]

use rstest::rstest;

use super::{generation_scenarios, scenarios};

#[rstest]
#[case::deployment_body_drops_model(scenarios::deployment_body_drops_model)]
#[case::v1_keeps_model(scenarios::v1_keeps_model)]
#[case::dalle2_dalle3_gpt_selection(scenarios::dalle2_dalle3_gpt_selection)]
#[case::image_edit_deployment_url(scenarios::image_edit_deployment_url)]
#[case::preview_image_path(scenarios::preview_image_path)]
#[case::flux2_and_mai_target(scenarios::flux2_and_mai_target)]
#[case::configured_base_query(scenarios::configured_base_query)]
#[case::azure_image_body_and_retry(generation_scenarios::azure_image_body_and_retry)]
#[case::azure_image_auth(generation_scenarios::azure_image_auth)]
#[case::azure_image_errors(generation_scenarios::azure_image_errors)]
fn images_cases(#[case] scenario: fn()) {
    scenario();
}
