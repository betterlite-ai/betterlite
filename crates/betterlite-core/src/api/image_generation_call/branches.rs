use betterlite_context::non_default_params::names::is_openai_compatible_provider;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageResponse, ImageUsage};

use super::call::ImageGenerationCall;
use super::error::ImageGenerationException;
use super::operations::{BranchAnswer, Headers, ImageGenerationOperations};
use super::plan::{ImageGenerationPlan, MappedOptions, PreparedCall};

mod azure;
mod custom;
mod foundry;
mod generic;
mod openai;

// the twelve providers that go through llm_http_handler.image_generation_handler
const GENERIC_PROVIDERS: [&str; 12] = [
    "recraft",
    "aiml",
    "gemini",
    "fal_ai",
    "stability",
    "runwayml",
    "vertex_ai",
    "openrouter",
    "dashscope",
    "qwencloud",
    "qwen_ai_platform",
    "edenai",
];

pub(super) async fn send_branch<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<BranchAnswer, ImageGenerationException>
where
    Operations: ImageGenerationOperations + Sync,
{
    let slug = prepared.provider_slug.as_str();
    if slug == "azure" {
        return azure::send(plan, prepared, mapped, call, logging).await;
    }
    if GENERIC_PROVIDERS.contains(&slug) {
        return generic::send(plan, prepared, mapped, call, logging).await;
    }
    if slug == "black_forest_labs" {
        return openai::black_forest_labs(plan, prepared, mapped, call, logging).await;
    }
    if slug == "azure_ai" {
        return foundry::send(plan, prepared, mapped, call, logging).await;
    }
    if slug == "openai" || slug == "litellm_proxy" || is_openai_compatible_provider(slug) {
        return openai::send(plan, prepared, mapped, call, logging).await;
    }
    if slug == "bedrock" {
        return openai::bedrock(plan, prepared, mapped, call, logging).await;
    }
    if plan.custom_providers.contains(slug) {
        return custom::send(plan, prepared, mapped, call, logging).await;
    }
    Ok(fallthrough(call))
}

// The response ImageResponse() starts with, returned when no branch handles the provider
fn fallthrough(call: &ImageGenerationCall) -> BranchAnswer {
    BranchAnswer {
        response: ImageResponse {
            created: call.now_seconds,
            data: Some(Vec::new()),
            usage: Some(ImageUsage::default()),
        },
        hidden_model: None,
    }
}

// extra_headers stays in optional_params for these handlers and is also merged into the request headers
pub(super) fn header_object(headers: &Headers) -> Json {
    Json::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}
