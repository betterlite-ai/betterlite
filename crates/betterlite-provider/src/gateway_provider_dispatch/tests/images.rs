use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_openai::openrouter_image::edit::get_openrouter_image_edit_config;
use betterlite_openai::openrouter_image::generation::get_openrouter_image_generation_config;
use rstest::rstest;

use crate::gateway_provider_dispatch::images::{image_edit_config, image_generation_config};

const MODEL: &str = "google/gemini-2.5-flash-image-preview";

#[test]
fn the_openrouter_generation_tag_gets_the_openrouter_config() {
    let tag = ImageGenerationConfigTag::OpenRouter(String::from(MODEL));
    assert_eq!(
        image_generation_config(&tag),
        Some(get_openrouter_image_generation_config(MODEL))
    );
}

#[test]
fn the_openrouter_edit_tag_gets_the_openrouter_config() {
    let tag = ImageEditConfigTag::OpenRouter(String::from(MODEL));
    assert_eq!(
        image_edit_config(&tag),
        Some(get_openrouter_image_edit_config(MODEL))
    );
}

#[rstest]
#[case::openai(ImageGenerationConfigTag::OpenAi(String::from(MODEL)))]
#[case::azure(ImageGenerationConfigTag::Azure(String::from(MODEL)))]
#[case::gemini(ImageGenerationConfigTag::Gemini(String::from(MODEL)))]
#[case::proxy(ImageGenerationConfigTag::LiteLlmProxy)]
#[case::eden(ImageGenerationConfigTag::EdenAi)]
fn other_generation_tags_get_no_config(#[case] tag: ImageGenerationConfigTag) {
    assert_eq!(image_generation_config(&tag), None);
}

#[rstest]
#[case::openai(ImageEditConfigTag::OpenAi(String::from(MODEL)))]
#[case::hosted_vllm(ImageEditConfigTag::HostedVllm(String::from(MODEL)))]
#[case::azure(ImageEditConfigTag::Azure)]
#[case::bedrock(ImageEditConfigTag::Bedrock(String::from(MODEL)))]
fn other_edit_tags_get_no_config(#[case] tag: ImageEditConfigTag) {
    assert_eq!(image_edit_config(&tag), None);
}
