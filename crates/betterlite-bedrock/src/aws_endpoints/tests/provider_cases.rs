use crate::aws_endpoints::provider::{
    BedrockEmbeddingProvider, BedrockInvokeProvider, embedding_provider, invoke_provider,
};

const INVOKE_CASES: [(&str, Option<&str>); 23] = [
    (
        "invoke/anthropic.claude-3-5-sonnet-20240620-v1:0",
        Some("anthropic"),
    ),
    ("us.amazon.nova-pro-v1:0", Some("nova")),
    ("amazon.nova-lite-v1:0", Some("nova")),
    ("amazon.titan-text-express-v1", Some("amazon")),
    ("meta.llama3-70b-instruct-v1:0", Some("meta")),
    ("us.meta.llama3-1-70b-instruct-v1:0", Some("meta")),
    (
        "llama/arn:aws:bedrock:us-east-1:1:imported-model/x",
        Some("llama"),
    ),
    ("anthropic.claude-3", Some("anthropic")),
    ("cohere.command-r-v1:0", Some("cohere")),
    ("mistral.mistral-7b-instruct-v0:2", Some("mistral")),
    ("ai21.j2-mid-v1", Some("ai21")),
    ("deepseek_r1/model", Some("deepseek_r1")),
    ("qwen3/coder", Some("qwen3")),
    ("qwen2/instruct", Some("qwen2")),
    ("twelvelabs/marengo", Some("twelvelabs")),
    ("openai.gpt-oss-20b-1:0", Some("openai")),
    ("stability.stable-diffusion-xl-v1", Some("stability")),
    ("moonshot/kimi", Some("moonshot")),
    ("x.cohere", Some("cohere")),
    ("invoke/invoke/anthropic.claude", Some("anthropic")),
    ("NOVA-model", Some("nova")),
    ("SomeAnthropicModel", None),
    ("unknown-model", None),
];

const EMBEDDING_CASES: [(&str, Option<&str>); 14] = [
    ("cohere.embed-english-v3:0", Some("cohere")),
    ("amazon.titan-embed-text-v1", Some("amazon")),
    ("amazon.nova-2-multimodal-embeddings-v1:0", Some("nova")),
    ("us.twelvelabs.marengo-embed-2-7-v1:0", Some("twelvelabs")),
    ("twelvelabs.marengo-embed-2-7-v1:0", Some("twelvelabs")),
    ("some-amazon-model", Some("amazon")),
    ("vendor/cohere-embed", Some("cohere")),
    ("Amazon.titan-embed-text-v1", None),
    ("NOVA-embed", Some("nova")),
    ("us.amazon.nova-lite-v1:0", Some("nova")),
    ("vendor/unknown", None),
    ("anthropic.claude-3", None),
    ("cohere.twelvelabs.x", Some("twelvelabs")),
    ("vendor-amazon-cohere", Some("cohere")),
];

pub(super) fn provider_selection_order() {
    for (model, expected) in INVOKE_CASES {
        assert_eq!(
            invoke_provider(model).map(BedrockInvokeProvider::name),
            expected,
            "{model}"
        );
    }
    for (model, expected) in EMBEDDING_CASES {
        assert_eq!(
            embedding_provider(model).map(BedrockEmbeddingProvider::name),
            expected,
            "{model}"
        );
    }
}
