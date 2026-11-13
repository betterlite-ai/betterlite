use betterlite_models::identity::provider_id::ProviderId;

use super::super::foundry::{self, AzureAiRoute};
use super::super::tags::{ChatConfigTag, ConfigConstruction};
use super::harness::{CHAT_CATALOG, CallSpec, TEXT_CATALOG, make_world};

const FOUNDRY_ROUTES: &[(&str, AzureAiRoute)] = &[
    ("agents/agent-id", AzureAiRoute::Agents),
    ("azure_ai/agents/agent-id", AzureAiRoute::Agents),
    ("AGENTS/agent-id", AzureAiRoute::Default),
    ("model_router/gpt-4o", AzureAiRoute::ModelRouter),
    ("MODEL_ROUTER/gpt-4o", AzureAiRoute::ModelRouter),
    ("model-router/gpt-4o", AzureAiRoute::ModelRouter),
    ("my-model-router-deployment", AzureAiRoute::ModelRouter),
    ("model_router", AzureAiRoute::ModelRouter),
    ("claude-sonnet", AzureAiRoute::Default),
    ("plain-deployment", AzureAiRoute::Default),
];

const FOUNDRY_SELECTIONS: &[(&str, ChatConfigTag, &str)] = &[
    (
        "model_router/claude",
        ChatConfigTag::AzureModelRouter,
        "AzureModelRouterConfig",
    ),
    (
        "MODEL-ROUTER/claude",
        ChatConfigTag::AzureModelRouter,
        "AzureModelRouterConfig",
    ),
    (
        "claude-sonnet",
        ChatConfigTag::AzureAnthropic,
        "AzureAnthropicConfig",
    ),
    (
        "CLAUDE-sonnet",
        ChatConfigTag::AzureAnthropic,
        "AzureAnthropicConfig",
    ),
    (
        "agents/claude/agent-id",
        ChatConfigTag::AzureAnthropic,
        "AzureAnthropicConfig",
    ),
    (
        "agents/gpt-4o",
        ChatConfigTag::AzureAiStudio,
        "AzureAIStudioConfig",
    ),
    (
        "gpt-4o",
        ChatConfigTag::AzureAiStudio,
        "AzureAIStudioConfig",
    ),
];

pub(super) fn foundry_selection() {
    let world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    for &(model, route) in FOUNDRY_ROUTES {
        assert_eq!(foundry::get_azure_ai_route(model), route, "{model}");
    }
    for &(model, tag, class) in FOUNDRY_SELECTIONS {
        let selection = world
            .select(&CallSpec::plain(ProviderId::AzureAi, model))
            .unwrap()
            .unwrap();
        assert_eq!(selection.tag, tag, "{model}");
        assert_eq!(selection.tag.class_name(), class, "{model}");
        assert_eq!(
            selection.construction,
            ConfigConstruction::New(vec![model.to_owned()]),
            "{model}"
        );
    }
}
