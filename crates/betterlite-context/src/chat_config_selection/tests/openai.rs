use betterlite_models::identity::provider_id::ProviderId;

use super::super::tags::ChatConfigTag;
use super::harness::{
    Arguments, CHAT_CATALOG, Case, TEXT_CATALOG, check, check_classes, make_world,
};

const OPENAI_CASES: &[Case] = &[
    Case {
        provider: ProviderId::Openai,
        model: "o1",
        base_model: None,
        tag: ChatConfigTag::OpenAiOSeries,
        arguments: Arguments::Shared,
    },
    Case {
        provider: ProviderId::Openai,
        model: "openai/o1",
        base_model: None,
        tag: ChatConfigTag::OpenAiOSeries,
        arguments: Arguments::Shared,
    },
    Case {
        provider: ProviderId::Openai,
        model: "o3-mini",
        base_model: None,
        tag: ChatConfigTag::OpenAiOSeries,
        arguments: Arguments::Shared,
    },
    Case {
        provider: ProviderId::Openai,
        model: "o4-mini",
        base_model: None,
        tag: ChatConfigTag::OpenAiOSeries,
        arguments: Arguments::Shared,
    },
    Case {
        provider: ProviderId::Openai,
        model: "o1-preview",
        base_model: None,
        tag: ChatConfigTag::OpenAiUnknownModel,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "o_series/foo",
        base_model: None,
        tag: ChatConfigTag::OpenAiUnknownModel,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "O1",
        base_model: None,
        tag: ChatConfigTag::OpenAiUnknownModel,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "o5",
        base_model: None,
        tag: ChatConfigTag::OpenAiUnknownModel,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "gpt-5",
        base_model: None,
        tag: ChatConfigTag::OpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "gpt-6",
        base_model: None,
        tag: ChatConfigTag::OpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "responses/gpt-5",
        base_model: None,
        tag: ChatConfigTag::OpenAiGpt5,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "gpt-4o",
        base_model: None,
        tag: ChatConfigTag::OpenAiGpt,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "gpt-5-chat-latest",
        base_model: None,
        tag: ChatConfigTag::OpenAiGpt,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "responses/gpt-4o",
        base_model: None,
        tag: ChatConfigTag::OpenAiGpt,
        arguments: Arguments::None,
    },
    Case {
        provider: ProviderId::Openai,
        model: "gpt-3.5-turbo-instruct",
        base_model: None,
        tag: ChatConfigTag::OpenAiGpt,
        arguments: Arguments::None,
    },
];

const OPENAI_CLASSES: &[(ProviderId, &str, &str)] = &[
    (ProviderId::Openai, "o1", "OpenAIOSeriesConfig"),
    (ProviderId::Openai, "gpt-5", "OpenAIGPT5Config"),
    (ProviderId::Openai, "missing", "OpenAIUnknownModelConfig"),
    (ProviderId::Openai, "gpt-4o", "OpenAIGPTConfig"),
];

pub(super) fn openai_family_selection() {
    let world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    check(&world, OPENAI_CASES);
    check_classes(&world, OPENAI_CLASSES);
    let without_chat = make_world(&["o1", "gpt-4o"], &[]);
    check(
        &without_chat,
        &[Case {
            provider: ProviderId::Openai,
            model: "gpt-5-chat-latest",
            base_model: None,
            tag: ChatConfigTag::OpenAiUnknownModel,
            arguments: Arguments::None,
        }],
    );
}
