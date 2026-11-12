use crate::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};

const PINNED_ORDER: [&str; 15] = [
    "cohere",
    "anthropic",
    "mistral",
    "amazon",
    "meta",
    "llama",
    "ai21",
    "nova",
    "deepseek_r1",
    "qwen3",
    "qwen2",
    "twelvelabs",
    "openai",
    "stability",
    "moonshot",
];

const CASES: [(&str, Option<&str>); 22] = [
    (
        "anthropic.claude-3-5-sonnet-20240620-v1:0",
        Some("anthropic"),
    ),
    (
        "invoke/anthropic.claude-3-5-sonnet-20240620-v1:0",
        Some("anthropic"),
    ),
    ("invoke/invoke/anthropic.claude-v2", Some("anthropic")),
    ("anthropic/model-name", Some("anthropic")),
    ("custom/anthropic-model", Some("anthropic")),
    ("us.amazon.nova-pro-v1:0", Some("nova")),
    ("AMAZON.NOVA-LITE-V1:0", Some("nova")),
    ("amazon.titan-text-express-v1", Some("amazon")),
    (
        "llama/arn:aws:bedrock:us-east-1:1:imported-model/x",
        Some("llama"),
    ),
    ("meta.llama3-8b-instruct-v1:0", Some("meta")),
    ("deepseek_r1/deepseek.r1-v1:0", Some("deepseek_r1")),
    ("qwen.qwen3-32b-v1:0", Some("qwen3")),
    ("qwen.qwen2-72b-v1:0", Some("qwen2")),
    ("twelvelabs.marengo-embed-2-7-v1:0", Some("twelvelabs")),
    ("openai.gpt-oss-20b", Some("openai")),
    ("stability.stable-image-core-v1:1", Some("stability")),
    ("moonshot.kimi-k2", Some("moonshot")),
    ("ai21.j2-mid-v1", Some("ai21")),
    ("cohere.command-r-v1:0", Some("cohere")),
    ("mistral.mixtral-8x7b-instruct", Some("mistral")),
    ("unmatched-model", None),
    ("", None),
];

pub(super) fn provider_family_selection() {
    assert_eq!(
        BedrockInvokeProvider::ALL.map(BedrockInvokeProvider::name),
        PINNED_ORDER
    );
    for (model, expected) in CASES {
        assert_eq!(
            invoke_provider(model).map(BedrockInvokeProvider::name),
            expected,
            "{model}"
        );
    }
}
