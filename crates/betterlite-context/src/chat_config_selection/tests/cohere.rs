use betterlite_models::identity::provider_id::ProviderId;

use crate::bedrock_config_selection::chat::{
    BedrockChatConfigTag, BedrockInvokeProvider, BedrockRoute,
};

use super::super::cohere;
use super::super::facts::BedrockFacts;
use super::super::tags::{ChatConfigTag, ConfigConstruction};
use super::harness::{
    Arguments, CHAT_CATALOG, CallSpec, Case, TEXT_CATALOG, World, check, make_world,
};

const COHERE_CASES: &[(&str, ChatConfigTag)] = &[
    ("v1/command", ChatConfigTag::CohereChat),
    ("cohere_chat/v1/command-r", ChatConfigTag::CohereChat),
    ("command-r", ChatConfigTag::CohereV2Chat),
    ("V1/command", ChatConfigTag::CohereV2Chat),
    ("v2/command", ChatConfigTag::CohereV2Chat),
];

const COHERE_CONFIG_CASES: &[Case] = &[
    Case {
        provider: ProviderId::Cohere,
        model: "v1/command",
        base_model: None,
        tag: ChatConfigTag::CohereChat,
        arguments: Arguments::Model,
    },
    Case {
        provider: ProviderId::CohereChat,
        model: "v1/command",
        base_model: None,
        tag: ChatConfigTag::CohereChat,
        arguments: Arguments::Model,
    },
    Case {
        provider: ProviderId::Cohere,
        model: "command-r",
        base_model: None,
        tag: ChatConfigTag::CohereV2Chat,
        arguments: Arguments::Model,
    },
    Case {
        provider: ProviderId::CohereChat,
        model: "V1/command",
        base_model: None,
        tag: ChatConfigTag::CohereV2Chat,
        arguments: Arguments::Model,
    },
];

struct BedrockCase {
    model: &'static str,
    route: BedrockRoute,
    invoke_provider: Option<BedrockInvokeProvider>,
    base_model: &'static str,
    tag: BedrockChatConfigTag,
    class: &'static str,
}

const BEDROCK_CASES: &[BedrockCase] = &[
    BedrockCase {
        model: "anthropic.claude-v2",
        route: BedrockRoute::Invoke,
        invoke_provider: Some(BedrockInvokeProvider::Anthropic),
        base_model: "anthropic.claude-v2",
        tag: BedrockChatConfigTag::Anthropic,
        class: "AmazonAnthropicConfig",
    },
    BedrockCase {
        model: "amazon.titan-text-express-v1",
        route: BedrockRoute::Invoke,
        invoke_provider: Some(BedrockInvokeProvider::Amazon),
        base_model: "amazon.titan-text-express-v1",
        tag: BedrockChatConfigTag::Titan,
        class: "AmazonTitanConfig",
    },
    BedrockCase {
        model: "converse/anthropic.claude-3",
        route: BedrockRoute::Converse,
        invoke_provider: None,
        base_model: "anthropic.claude-3",
        tag: BedrockChatConfigTag::Converse,
        class: "AmazonConverseConfig",
    },
];

pub(super) fn cohere_and_bedrock_selection() {
    let world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    cohere_selection(&world);
    assert_eq!(cohere::get_cohere_route("v1/x"), cohere::CohereRoute::V1);
    assert_eq!(cohere::get_cohere_route("v2/x"), cohere::CohereRoute::V2);
    bedrock_selection(&world);
    helper_factories(&world);
}

fn cohere_selection(world: &World) {
    check(world, COHERE_CONFIG_CASES);
    for &(model, tag) in COHERE_CASES {
        assert_eq!(cohere::select_config(model), tag, "{model}");
        assert_eq!(
            world
                .select(&CallSpec::plain(ProviderId::Cohere, model))
                .unwrap()
                .map(|selection| selection.tag),
            Some(tag),
            "{model}"
        );
    }
}

fn bedrock_selection(world: &World) {
    let selected: Vec<(ChatConfigTag, &str, ConfigConstruction)> = BEDROCK_CASES
        .iter()
        .map(|case| {
            let spec = CallSpec {
                provider: ProviderId::Bedrock,
                model: case.model,
                base_model: None,
                bedrock: BedrockFacts {
                    route: case.route,
                    invoke_provider: case.invoke_provider,
                    base_model: case.base_model,
                },
            };
            let selection = world.select(&spec).unwrap().unwrap();
            (
                selection.tag,
                selection.tag.class_name(),
                selection.construction,
            )
        })
        .collect();
    let expected: Vec<(ChatConfigTag, &str, ConfigConstruction)> = BEDROCK_CASES
        .iter()
        .map(|case| {
            (
                ChatConfigTag::Bedrock(case.tag),
                case.class,
                ConfigConstruction::New(vec![case.model.to_owned()]),
            )
        })
        .collect();
    assert_eq!(selected, expected);
}

fn helper_factories(world: &World) {
    let helpers: &[(ProviderId, ChatConfigTag, &str)] = &[
        (ProviderId::Sail, ChatConfigTag::SailChat, "SailChatConfig"),
        (
            ProviderId::Langgraph,
            ChatConfigTag::LangGraph,
            "LangGraphConfig",
        ),
        (
            ProviderId::Langflow,
            ChatConfigTag::LangFlow,
            "LangFlowConfig",
        ),
    ];
    for &(provider, tag, class) in helpers {
        let selection = world
            .select(&CallSpec::plain(provider, "test-model"))
            .unwrap()
            .unwrap();
        assert_eq!(selection.tag, tag, "{provider:?}");
        assert_eq!(selection.tag.class_name(), class, "{provider:?}");
        assert_eq!(
            selection.construction,
            ConfigConstruction::New(Vec::new()),
            "{provider:?}"
        );
    }
}
