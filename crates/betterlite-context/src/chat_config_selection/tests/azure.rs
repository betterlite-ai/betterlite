use betterlite_models::identity::provider_id::ProviderId;

use super::super::azure;
use super::super::tags::ChatConfigTag;
use super::harness::{
    Arguments, CHAT_CATALOG, Case, TEXT_CATALOG, check, check_classes, make_world,
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
];

const AZURE_CLASSES: &[(ProviderId, &str, &str)] = &[
    (ProviderId::Azure, "o1", "AzureOpenAIO1Config"),
    (ProviderId::Azure, "gpt-5", "AzureOpenAIGPT5Config"),
    (ProviderId::Azure, "gpt-4o", "AzureOpenAIConfig"),
];

pub(super) fn azure_detection_selection() {
    let world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    check(&world, AZURE_CASES);
    check_classes(&world, AZURE_CLASSES);
    for model in ["o1", "o3", "o4", "o_series/x", "foo-o1-bar", "o1-preview"] {
        assert!(azure::is_o_series_model(model), "{model}");
    }
    for model in ["O1", "o", "gpt-4o", "o_series", "o_seriesx", "oo"] {
        assert!(!azure::is_o_series_model(model), "{model}");
    }
}
