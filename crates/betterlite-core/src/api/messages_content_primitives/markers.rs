use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::arguments::shapes::with_prompt_cache_breakpoint;

// ":bedrock:" is the ARN service field, so a sagemaker endpoint that merely names bedrock stays out.
#[must_use]
pub fn target_consumes_cache_control(model: &str) -> bool {
    let lowered = model.to_lowercase();
    let claude = lowered.contains("anthropic") || lowered.contains("claude");
    let bedrock_arn = lowered.contains("arn:") && lowered.contains(":bedrock:");
    claude || bedrock_arn || lowered.contains("gemini")
}

pub fn copy_cache_control(source: &Object, target: &mut Object, model: Option<&str>) {
    let Some(cache_control) = source
        .get("cache_control")
        .filter(|control| control.is_truthy())
    else {
        return;
    };
    if model.is_some_and(|name| !name.is_empty() && target_consumes_cache_control(name)) {
        target.insert("cache_control".to_owned(), cache_control.clone());
    }
}

// A null marker is Python's None, which B216 drops; false and 0 are markers and stay.
#[must_use]
pub fn carry_cache_breakpoint(source: &Object, target: &Object) -> Object {
    match source.get("prompt_cache_breakpoint") {
        None | Some(Json::Null) => target.clone(),
        marker => with_prompt_cache_breakpoint(target, marker),
    }
}

pub async fn mid_system_supported<Predicates>(
    predicates: &mut Predicates,
    model: Option<&str>,
    custom_llm_provider: Option<&str>,
) -> bool
where
    Predicates: CapabilityPredicates + Send,
{
    let Some(name) = model.filter(|name| !name.is_empty()) else {
        return false;
    };
    let request = CapabilityRequest {
        model: name,
        custom_llm_provider,
    };
    let key = NamedCapability::MidConversationSystem.key();
    predicates.supports_factory(&request, key).await
}
