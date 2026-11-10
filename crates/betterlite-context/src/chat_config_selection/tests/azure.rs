use betterlite_models::identity::provider_id::ProviderId;

use crate::bedrock_config_selection::chat::{
    BedrockChatConfigTag, BedrockInvokeProvider, BedrockRoute,
};

use super::super::azure;
use super::super::facts::BedrockFacts;
use super::super::tags::{ChatConfigTag, ConfigConstruction};
use super::harness::{
    Arguments, CHAT_CATALOG, CallSpec, Case, TEXT_CATALOG, check, check_classes, make_world,
};

const AZURE_CASES: &[Case] = &[
    Case {
        provider: ProviderId::Azure,
        model: "o1",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "o3-mini",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "o4",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "o_series/deployment",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "deployment-o1-name",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "my-deployment",
        base_model: Some("o3-mini"),
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "o1",
        base_model: Some("gpt-5"),
        tag: ChatConfigTag::AzureOpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt-5",
        base_model: Some("plain-deployment"),
        tag: ChatConfigTag::AzureOpenAi,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "o1",
        base_model: Some(""),
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt-5",
        base_model: Some(""),
        tag: ChatConfigTag::AzureOpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt-5",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt-6",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt-5-chat",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAi,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt5_series-deployment",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "O1",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAi,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "o_series",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAi,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt-4o",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAi,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "my-deployment",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAi,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "gpt-5-o3-eval",
        base_model: None,
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Azure,
        model: "my-deployment",
        base_model: Some("gpt-5-o1"),
        tag: ChatConfigTag::AzureOpenAiO1,
        arguments: Arguments::None,
    },
];

const NON_AZURE_BASE_MODEL_CASES: &[Case] = &[
    Case {
        provider: ProviderId::Openai,
        model: "gpt-4o",
        base_model: Some("o1"),
        tag: ChatConfigTag::OpenAiGpt,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "gpt-4o",
        base_model: Some("gpt-5"),
        tag: ChatConfigTag::OpenAiGpt,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::AzureAi,
        model: "gpt-4o",
        base_model: Some("claude-x"),
        tag: ChatConfigTag::AzureAiStudio,
        arguments: Arguments::Model,
    },
    Case {
        provider: ProviderId::VertexAi,
        model: "llama-3-70b",
        base_model: Some("gemini-pro"),
        tag: ChatConfigTag::VertexAiLlama3,
        arguments: Arguments::Model,
    },
    Case {
        provider: ProviderId::Cohere,
        model: "command-r",
        base_model: Some("v1/command"),
        tag: ChatConfigTag::CohereV2Chat,
        arguments: Arguments::Model,
    },
    Case {
        provider: ProviderId::CohereChat,
        model: "v1/command",
        base_model: Some("command-r"),
        tag: ChatConfigTag::CohereChat,
        arguments: Arguments::Model,
    },
];

const AZURE_CLASSES: &[(ProviderId, &str, &str)] = &[
    (ProviderId::Azure, "o1", "AzureOpenAIO1Config"),
    (ProviderId::Azure, "gpt-5", "AzureOpenAIGPT5Config"),
    (ProviderId::Azure, "gpt-4o", "AzureOpenAIConfig"),
];

pub(super) fn azure_detection_selection() {
    let world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    check(&world, AZURE_CASES);
    check(&world, NON_AZURE_BASE_MODEL_CASES);
    bedrock_ignores_base_model(&world);
    check_classes(&world, AZURE_CLASSES);
    for model in ["o1", "o3", "o4", "o_series/x", "foo-o1-bar", "o1-preview"] {
        assert!(azure::is_o_series_model(model), "{model}");
    }
    for model in ["O1", "o", "gpt-4o", "o_series", "o_seriesx", "oo"] {
        assert!(!azure::is_o_series_model(model), "{model}");
    }
}

fn bedrock_ignores_base_model(world: &super::harness::World) {
    let spec = CallSpec {
        provider: ProviderId::Bedrock,
        model: "anthropic.claude-v2",
        base_model: Some("converse/other"),
        bedrock: BedrockFacts {
            route: BedrockRoute::Invoke,
            invoke_provider: Some(BedrockInvokeProvider::Anthropic),
            base_model: "anthropic.claude-v2",
        },
    };
    let selection = world.select(&spec).unwrap().unwrap();
    assert_eq!(
        selection.tag,
        ChatConfigTag::Bedrock(BedrockChatConfigTag::Anthropic)
    );
    assert_eq!(
        selection.construction,
        ConfigConstruction::New(vec!["anthropic.claude-v2".to_owned()])
    );
}
