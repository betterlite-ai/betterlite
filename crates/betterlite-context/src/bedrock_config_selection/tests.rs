#![cfg(test)]

use rstest::rstest;

use super::chat::{
    BedrockChatConfigTag as ChatTag, BedrockInvokeProvider as Provider, BedrockRoute as Route,
    select_bedrock_chat_config,
};
use super::legacy::LEGACY_ANTHROPIC_MODEL_NAMES;
use super::messages::{
    BedrockMessagesConfigTag as MessagesTag, ExplicitRoute, select_bedrock_messages_config,
};

const NO_ROUTES: &[ExplicitRoute] = &[];

const NATIVE_CASES: &[(&[ExplicitRoute], &str, Option<MessagesTag>)] = &[
    (
        &[
            ExplicitRoute::ClaudePlatform,
            ExplicitRoute::Converse,
            ExplicitRoute::Mantle,
        ],
        "converse/claude",
        Some(MessagesTag::ClaudePlatform),
    ),
    (
        &[ExplicitRoute::Converse, ExplicitRoute::Mantle],
        "mantle/claude",
        None,
    ),
    (
        &[ExplicitRoute::Mantle],
        "mantle/claude",
        Some(MessagesTag::Mantle),
    ),
    (
        NO_ROUTES,
        "anthropic.claude-3",
        Some(MessagesTag::AnthropicClaude),
    ),
    (NO_ROUTES, "anthropic.Claude-3", None),
    (NO_ROUTES, "CLAUDE-3", None),
    (
        NO_ROUTES,
        "my-claude-model",
        Some(MessagesTag::AnthropicClaude),
    ),
    (NO_ROUTES, "amazon.titan-text-express-v1", None),
    (
        NO_ROUTES,
        "converse_like/anthropic.claude-3",
        Some(MessagesTag::AnthropicClaude),
    ),
];

const EXPLICIT_ROUTE_CASES: &[(Route, Option<Provider>, &str, ChatTag)] = &[
    (
        Route::ClaudePlatform,
        Some(Provider::Amazon),
        "anthropic.claude-v2",
        ChatTag::ClaudePlatform,
    ),
    (
        Route::Converse,
        Some(Provider::Amazon),
        "amazon.titan-text-express-v1",
        ChatTag::Converse,
    ),
    (Route::ConverseLike, None, "any", ChatTag::Converse),
    (Route::Openai, None, "openai.gpt-5.6", ChatTag::OpenAi),
    (
        Route::ChatCompletions,
        Some(Provider::Anthropic),
        "anthropic.claude-3",
        ChatTag::RuntimeChatCompletions,
    ),
    (
        Route::Agent,
        Some(Provider::Amazon),
        "any",
        ChatTag::InvokeAgent,
    ),
    (Route::Agentcore, None, "any", ChatTag::AgentCore),
    (
        Route::Mantle,
        Some(Provider::Amazon),
        "any",
        ChatTag::Mantle,
    ),
];

const PROVIDER_CASES: &[(Route, Option<Provider>, &str, ChatTag)] = &[
    (
        Route::AsyncInvoke,
        Some(Provider::Amazon),
        "amazon.titan-text-express-v1",
        ChatTag::Titan,
    ),
    (
        Route::Invoke,
        Some(Provider::Amazon),
        "amazon.titan-text-express-v1",
        ChatTag::Titan,
    ),
    (
        Route::Invoke,
        Some(Provider::Anthropic),
        "anthropic.claude-v2",
        ChatTag::Anthropic,
    ),
    (
        Route::Invoke,
        Some(Provider::Anthropic),
        "anthropic.claude-instant-v1",
        ChatTag::Anthropic,
    ),
    (
        Route::Invoke,
        Some(Provider::Anthropic),
        "anthropic.claude-v2:1",
        ChatTag::Anthropic,
    ),
    (
        Route::Invoke,
        Some(Provider::Anthropic),
        "anthropic.claude-v2:0",
        ChatTag::AnthropicClaude,
    ),
    (
        Route::Invoke,
        Some(Provider::Anthropic),
        "anthropic.claude-3-5-sonnet-20240620-v1:0",
        ChatTag::AnthropicClaude,
    ),
    (
        Route::Invoke,
        Some(Provider::Meta),
        "meta.llama3-8b-instruct-v1:0",
        ChatTag::Llama,
    ),
    (
        Route::Invoke,
        Some(Provider::Llama),
        "llama/imported-model/x",
        ChatTag::Llama,
    ),
    (
        Route::Invoke,
        Some(Provider::Ai21),
        "ai21.j2-mid-v1",
        ChatTag::Ai21,
    ),
    (
        Route::Invoke,
        Some(Provider::Cohere),
        "cohere.command-r-v1:0",
        ChatTag::Cohere,
    ),
    (
        Route::Invoke,
        Some(Provider::Mistral),
        "mistral.mistral-7b-instruct-v0:2",
        ChatTag::Mistral,
    ),
    (
        Route::Invoke,
        Some(Provider::Moonshot),
        "moonshot/foo",
        ChatTag::Moonshot,
    ),
    (
        Route::Invoke,
        Some(Provider::DeepseekR1),
        "deepseek_r1/foo",
        ChatTag::DeepSeekR1,
    ),
    (
        Route::Invoke,
        Some(Provider::Nova),
        "amazon.nova-pro-v1:0",
        ChatTag::Nova,
    ),
    (
        Route::Invoke,
        Some(Provider::Qwen3),
        "qwen3/foo",
        ChatTag::Qwen3,
    ),
    (
        Route::Invoke,
        Some(Provider::Qwen2),
        "qwen2/foo",
        ChatTag::Qwen2,
    ),
    (
        Route::Invoke,
        Some(Provider::Twelvelabs),
        "twelvelabs/pegasus",
        ChatTag::TwelveLabs,
    ),
    (
        Route::Invoke,
        Some(Provider::Openai),
        "openai.gpt-oss-120b",
        ChatTag::Invoke,
    ),
    (
        Route::Invoke,
        Some(Provider::Stability),
        "stability.foo",
        ChatTag::Invoke,
    ),
    (Route::Invoke, None, "foo.bar-baz", ChatTag::Invoke),
];

const LEGACY_CASES: &[(&str, ChatTag)] = &[
    ("anthropic.claude-v2", ChatTag::Anthropic),
    ("anthropic.claude-instant-v1", ChatTag::Anthropic),
    ("anthropic.claude-v2:1", ChatTag::Anthropic),
    ("anthropic.claude-v2:0", ChatTag::AnthropicClaude),
    ("anthropic.claude-v2:1:0k", ChatTag::AnthropicClaude),
    ("ANTHROPIC.CLAUDE-V2", ChatTag::AnthropicClaude),
    ("us.anthropic.claude-v2", ChatTag::AnthropicClaude),
];

const CONSTRUCTION_CHAT: &[(Route, Option<Provider>, &str, ChatTag)] = &[
    (Route::ClaudePlatform, None, "any", ChatTag::ClaudePlatform),
    (Route::Converse, None, "any", ChatTag::Converse),
    (Route::Openai, None, "any", ChatTag::OpenAi),
    (
        Route::ChatCompletions,
        None,
        "any",
        ChatTag::RuntimeChatCompletions,
    ),
    (Route::Agent, None, "any", ChatTag::InvokeAgent),
    (Route::Agentcore, None, "any", ChatTag::AgentCore),
    (Route::Mantle, None, "any", ChatTag::Mantle),
    (Route::Invoke, Some(Provider::Amazon), "any", ChatTag::Titan),
    (
        Route::Invoke,
        Some(Provider::Anthropic),
        "anthropic.claude-v2",
        ChatTag::Anthropic,
    ),
    (
        Route::Invoke,
        Some(Provider::Anthropic),
        "any",
        ChatTag::AnthropicClaude,
    ),
    (Route::Invoke, Some(Provider::Meta), "any", ChatTag::Llama),
    (Route::Invoke, Some(Provider::Ai21), "any", ChatTag::Ai21),
    (
        Route::Invoke,
        Some(Provider::Cohere),
        "any",
        ChatTag::Cohere,
    ),
    (
        Route::Invoke,
        Some(Provider::Mistral),
        "any",
        ChatTag::Mistral,
    ),
    (
        Route::Invoke,
        Some(Provider::Moonshot),
        "any",
        ChatTag::Moonshot,
    ),
    (
        Route::Invoke,
        Some(Provider::DeepseekR1),
        "any",
        ChatTag::DeepSeekR1,
    ),
    (Route::Invoke, Some(Provider::Nova), "any", ChatTag::Nova),
    (Route::Invoke, Some(Provider::Qwen3), "any", ChatTag::Qwen3),
    (Route::Invoke, Some(Provider::Qwen2), "any", ChatTag::Qwen2),
    (
        Route::Invoke,
        Some(Provider::Twelvelabs),
        "any",
        ChatTag::TwelveLabs,
    ),
    (Route::AsyncInvoke, None, "any", ChatTag::Invoke),
];

const CONSTRUCTION_MESSAGES: &[(&[ExplicitRoute], &str, Option<MessagesTag>)] = &[
    (
        &[ExplicitRoute::ClaudePlatform],
        "any",
        Some(MessagesTag::ClaudePlatform),
    ),
    (NO_ROUTES, "any-claude", Some(MessagesTag::AnthropicClaude)),
    (&[ExplicitRoute::Mantle], "any", Some(MessagesTag::Mantle)),
    (
        &[ExplicitRoute::Converse, ExplicitRoute::Mantle],
        "any-claude",
        None,
    ),
];

fn check_chat(cases: &[(Route, Option<Provider>, &str, ChatTag)]) {
    for &(route, provider, base_model, expected) in cases {
        assert_eq!(
            select_bedrock_chat_config(route, provider, base_model),
            expected,
            "{route:?} {provider:?} {base_model}"
        );
    }
}

fn check_messages(cases: &[(&[ExplicitRoute], &str, Option<MessagesTag>)]) {
    for &(explicit, model, expected) in cases {
        assert_eq!(
            select_bedrock_messages_config(model, explicit),
            expected,
            "{model}"
        );
    }
}

#[rstest]
#[case::native_messages_route(native_messages_route)]
#[case::chat_config_selector(chat_config_selector)]
#[case::legacy_anthropic_names(legacy_anthropic_names)]
#[case::selection_without_construction(selection_without_construction)]
fn bedrock_config_selection_cases(#[case] check: fn()) {
    check();
}

fn native_messages_route() {
    check_messages(NATIVE_CASES);
}

fn chat_config_selector() {
    check_chat(EXPLICIT_ROUTE_CASES);
    check_chat(PROVIDER_CASES);
}

fn legacy_anthropic_names() {
    assert_eq!(
        LEGACY_ANTHROPIC_MODEL_NAMES,
        [
            "anthropic.claude-v2",
            "anthropic.claude-instant-v1",
            "anthropic.claude-v2:1"
        ]
    );
    for &(base_model, expected) in LEGACY_CASES {
        assert_eq!(
            select_bedrock_chat_config(Route::Invoke, Some(Provider::Anthropic), base_model),
            expected,
            "{base_model}"
        );
    }
}

const CHAT_CLASS_NAMES: [&str; 21] = [
    "AmazonAI21Config",
    "AmazonAnthropicClaudeConfig",
    "AmazonAnthropicConfig",
    "AmazonBedrockOpenAIConfig",
    "AmazonBedrockRuntimeChatCompletionsConfig",
    "AmazonCohereConfig",
    "AmazonConverseConfig",
    "AmazonDeepSeekR1Config",
    "AmazonInvokeAgentConfig",
    "AmazonInvokeConfig",
    "AmazonInvokeNovaConfig",
    "AmazonLlamaConfig",
    "AmazonMantleConfig",
    "AmazonMistralConfig",
    "AmazonMoonshotConfig",
    "AmazonQwen2Config",
    "AmazonQwen3Config",
    "AmazonTitanConfig",
    "AmazonTwelveLabsPegasusConfig",
    "AmazonAgentCoreConfig",
    "BedrockClaudePlatformConfig",
];

const MESSAGES_CLASS_NAMES: [&str; 3] = [
    "AmazonAnthropicClaudeMessagesConfig",
    "AmazonMantleMessagesConfig",
    "BedrockClaudePlatformMessagesConfig",
];

fn selection_without_construction() {
    let mut classes: Vec<&str> = CONSTRUCTION_CHAT
        .iter()
        .map(|&(route, provider, base_model, expected)| {
            let tag = select_bedrock_chat_config(route, provider, base_model);
            assert_eq!(tag, expected, "{route:?} {provider:?} {base_model}");
            tag.class_name()
        })
        .collect();
    classes.sort_unstable();
    let mut expected_classes = CHAT_CLASS_NAMES;
    expected_classes.sort_unstable();
    assert_eq!(classes, expected_classes);

    let mut messages: Vec<&str> = CONSTRUCTION_MESSAGES
        .iter()
        .filter_map(|&(explicit, model, expected)| {
            let selected = select_bedrock_messages_config(model, explicit);
            assert_eq!(selected, expected, "{model}");
            selected.map(MessagesTag::class_name)
        })
        .collect();
    messages.sort_unstable();
    assert_eq!(messages, MESSAGES_CLASS_NAMES);
}
