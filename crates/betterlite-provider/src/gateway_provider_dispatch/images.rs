use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_openai::openrouter_image::edit::{
    OpenRouterImageEditConfig, get_openrouter_image_edit_config,
};
use betterlite_openai::openrouter_image::generation::{
    OpenRouterImageGenerationConfig, get_openrouter_image_generation_config,
};

// B5360 hands OpenRouter's tags here; main.py needs no extra code for these two operations
#[rustfmt::skip]
#[must_use]
pub fn image_generation_config(tag: &ImageGenerationConfigTag) -> Option<OpenRouterImageGenerationConfig> {
    match tag {
        ImageGenerationConfigTag::OpenRouter(model) => Some(get_openrouter_image_generation_config(model)),
        ImageGenerationConfigTag::OpenAi(_) | ImageGenerationConfigTag::Azure(_) | ImageGenerationConfigTag::AzureAi(_) | ImageGenerationConfigTag::Xinference(_)
        | ImageGenerationConfigTag::Recraft(_) | ImageGenerationConfigTag::Aiml(_) | ImageGenerationConfigTag::CometApi(_) | ImageGenerationConfigTag::Gemini(_) | ImageGenerationConfigTag::LiteLlmProxy
        | ImageGenerationConfigTag::FalAi(_) | ImageGenerationConfigTag::Stability(_) | ImageGenerationConfigTag::RunwayMl(_) | ImageGenerationConfigTag::BlackForestLabs(_) | ImageGenerationConfigTag::VertexAi(_)
        | ImageGenerationConfigTag::DashscopeFamily(_) | ImageGenerationConfigTag::ModelScope(_) | ImageGenerationConfigTag::EdenAi => None,
    }
}

#[rustfmt::skip]
#[must_use]
pub fn image_edit_config(tag: &ImageEditConfigTag) -> Option<OpenRouterImageEditConfig> {
    match tag {
        ImageEditConfigTag::OpenRouter(model) => Some(get_openrouter_image_edit_config(model)),
        ImageEditConfigTag::OpenAi(_) | ImageEditConfigTag::HostedVllm(_) | ImageEditConfigTag::Azure | ImageEditConfigTag::Recraft
        | ImageEditConfigTag::BlackForestLabs | ImageEditConfigTag::FalAi(_) | ImageEditConfigTag::AzureAi(_) | ImageEditConfigTag::Gemini(_)
        | ImageEditConfigTag::LiteLlmProxy | ImageEditConfigTag::VertexAi(_) | ImageEditConfigTag::Stability(_) | ImageEditConfigTag::Bedrock(_) => None,
    }
}
