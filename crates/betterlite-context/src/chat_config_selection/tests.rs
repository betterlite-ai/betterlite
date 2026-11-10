#![cfg(test)]

use rstest::rstest;

mod azure;
mod cohere;
mod factory;
mod foundry;
mod harness;
mod image;
mod json;
mod openai;
mod vertex;

#[rstest]
#[case::openai_family_selection(openai::openai_family_selection)]
#[case::azure_detection_selection(azure::azure_detection_selection)]
#[case::factory_table(factory::factory_table)]
#[case::foundry_selection(foundry::foundry_selection)]
#[case::vertex_selection(vertex::vertex_selection)]
#[case::cohere_and_bedrock_selection(cohere::cohere_and_bedrock_selection)]
#[case::json_fallback_precedence(json::json_fallback_precedence)]
#[case::image_variation_config_selection(image::image_variation_config_selection)]
fn chat_config_selection_cases(#[case] scenario: fn()) {
    scenario();
}
