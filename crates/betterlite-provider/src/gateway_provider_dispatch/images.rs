use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_openai::openrouter_image::edit::OpenRouterImageEditConfig;
// ast-grep-ignore: no-long-names
use betterlite_openai::openrouter_image::edit::get_openrouter_image_edit_config as edit_config;
// ast-grep-ignore: no-long-names
use betterlite_openai::openrouter_image::generation::OpenRouterImageGenerationConfig as GenerationConfig;
// ast-grep-ignore: no-long-names
use betterlite_openai::openrouter_image::generation::get_openrouter_image_generation_config as generation_config;

// the tag B5360 selects for either image operation; main.py builds no config of its own for them
pub enum ImageTag<'tag> {
    Generation(&'tag ImageGenerationConfigTag),
    Edit(&'tag ImageEditConfigTag),
}

// the config the OpenRouter arm builds; every other provider's config belongs to its own media dispatch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageConfig {
    Generation(GenerationConfig),
    Edit(OpenRouterImageEditConfig),
}

#[rustfmt::skip]
#[must_use]
pub const fn openrouter_config(tag: &ImageTag<'_>) -> Option<ImageConfig> {
    match tag {
        ImageTag::Generation(ImageGenerationConfigTag::OpenRouter(model)) => Some(ImageConfig::Generation(generation_config(model.as_str()))),
        ImageTag::Edit(ImageEditConfigTag::OpenRouter(model)) => Some(ImageConfig::Edit(edit_config(model.as_str()))),
        ImageTag::Generation(ImageGenerationConfigTag::OpenAi(_) | ImageGenerationConfigTag::Azure(_) | ImageGenerationConfigTag::AzureAi(_) | ImageGenerationConfigTag::Xinference(_)
        | ImageGenerationConfigTag::Recraft(_) | ImageGenerationConfigTag::Aiml(_) | ImageGenerationConfigTag::CometApi(_) | ImageGenerationConfigTag::Gemini(_) | ImageGenerationConfigTag::LiteLlmProxy
        | ImageGenerationConfigTag::FalAi(_) | ImageGenerationConfigTag::Stability(_) | ImageGenerationConfigTag::RunwayMl(_) | ImageGenerationConfigTag::BlackForestLabs(_) | ImageGenerationConfigTag::VertexAi(_)
        | ImageGenerationConfigTag::DashscopeFamily(_) | ImageGenerationConfigTag::ModelScope(_) | ImageGenerationConfigTag::EdenAi)
        | ImageTag::Edit(ImageEditConfigTag::OpenAi(_) | ImageEditConfigTag::HostedVllm(_) | ImageEditConfigTag::Azure | ImageEditConfigTag::Recraft
        | ImageEditConfigTag::BlackForestLabs | ImageEditConfigTag::FalAi(_) | ImageEditConfigTag::AzureAi(_) | ImageEditConfigTag::Gemini(_)
        | ImageEditConfigTag::LiteLlmProxy | ImageEditConfigTag::VertexAi(_) | ImageEditConfigTag::Stability(_) | ImageEditConfigTag::Bedrock(_)) => None,
    }
}
