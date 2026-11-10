use crate::identity::provider_id::ProviderId;

#[derive(Clone, Copy)]
pub(super) enum EndpointProvider {
    Anyscale,
    Known(ProviderId),
}

// LiteLLM keeps every one of these endpoints in openai_compatible_endpoints, in this order.
pub(super) const ENDPOINTS: [&str; 52] = [
    "api.perplexity.ai",
    "api.endpoints.anyscale.com/v1",
    "api.deepinfra.com/v1/openai",
    "api.mistral.ai/v1",
    "codestral.mistral.ai/v1/chat/completions",
    "codestral.mistral.ai/v1/fim/completions",
    "api.groq.com/openai/v1",
    "https://integrate.api.nvidia.com/v1",
    "https://api.getnadir.com/v1",
    "api.deepseek.com/v1",
    "api.together.ai/v1",
    "api.together.xyz/v1",
    "app.empower.dev/api/v1",
    "https://api.friendli.ai/serverless/v1",
    "api.sambanova.ai/v1",
    "api.x.ai/v1",
    "ollama.com",
    "api.galadriel.ai/v1",
    "api.llama.com/compat/v1/",
    "api.featherless.ai/v1",
    "inference.api.nscale.com/v1",
    "api.studio.nebius.ai/v1",
    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    "https://dashscope.aliyuncs.com/compatible-mode/v1",
    "https://api-inference.modelscope.cn/v1",
    "https://api.moonshot.ai/v1",
    "https://api.publicai.co/v1",
    "https://api.synthetic.new/openai/v1",
    "https://serverless.tensormesh.ai/v1",
    "https://api.stima.tech/v1",
    "https://nano-gpt.com/api/v1",
    "https://api.poe.com/v1",
    "https://llm.chutes.ai/v1/",
    "https://api.v0.dev/v1",
    "https://api.morphllm.com/v1",
    "https://api.lambda.ai/v1",
    "https://api.inceptionlabs.ai/v1",
    "https://api.hyperbolic.xyz/v1",
    "https://ai-gateway.helicone.ai/",
    "https://ai-gateway.vercel.sh/v1",
    "https://api.edenai.run/v3",
    "https://api.inference.wandb.ai/v1",
    "https://api.clarifai.com/v2/ext/openai/v1",
    "https://api.libertai.io/v1",
    "https://pinstripes.io/v1",
    "https://api.meta.ai/v1",
    "https://api.sailresearch.com/v1",
    "https://api.cognition.ai/v1",
    "https://api.cortecs.ai/v1",
    "https://api.scx.ai/v1",
    "https://api.prisminference.com/v1",
    "https://gigachat.devices.sberbank.ru/api/v1",
];

pub(super) enum KeySource {
    Env(&'static [&'static str]),
    EnvUnlessGiven(&'static [&'static str]),
    HttpsOnly(&'static [&'static str]),
}

pub(super) struct Branch {
    pub endpoint: &'static str,
    pub provider: EndpointProvider,
    pub keys: KeySource,
}

pub(super) const BRANCHES: &[Branch] = &[
    Branch {
        endpoint: "api.perplexity.ai",
        provider: EndpointProvider::Known(ProviderId::Perplexity),
        keys: KeySource::Env(&["PERPLEXITYAI_API_KEY"]),
    },
    Branch {
        endpoint: "api.endpoints.anyscale.com/v1",
        provider: EndpointProvider::Anyscale,
        keys: KeySource::Env(&["ANYSCALE_API_KEY"]),
    },
    Branch {
        endpoint: "api.deepinfra.com/v1/openai",
        provider: EndpointProvider::Known(ProviderId::Deepinfra),
        keys: KeySource::Env(&["DEEPINFRA_API_KEY"]),
    },
    Branch {
        endpoint: "api.mistral.ai/v1",
        provider: EndpointProvider::Known(ProviderId::Mistral),
        keys: KeySource::Env(&["MISTRAL_API_KEY"]),
    },
    Branch {
        endpoint: "codestral.mistral.ai/v1/chat/completions",
        provider: EndpointProvider::Known(ProviderId::Codestral),
        keys: KeySource::Env(&["CODESTRAL_API_KEY"]),
    },
    Branch {
        endpoint: "codestral.mistral.ai/v1/fim/completions",
        provider: EndpointProvider::Known(ProviderId::TextCompletionCodestral),
        keys: KeySource::Env(&["CODESTRAL_API_KEY"]),
    },
    Branch {
        endpoint: "api.groq.com/openai/v1",
        provider: EndpointProvider::Known(ProviderId::Groq),
        keys: KeySource::Env(&["GROQ_API_KEY"]),
    },
    Branch {
        endpoint: "https://integrate.api.nvidia.com/v1",
        provider: EndpointProvider::Known(ProviderId::NvidiaNim),
        keys: KeySource::Env(&["NVIDIA_NIM_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.getnadir.com/v1",
        provider: EndpointProvider::Known(ProviderId::Nadir),
        keys: KeySource::HttpsOnly(&["NADIR_API_KEY"]),
    },
    Branch {
        endpoint: "api.deepseek.com/v1",
        provider: EndpointProvider::Known(ProviderId::Deepseek),
        keys: KeySource::Env(&["DEEPSEEK_API_KEY"]),
    },
    Branch {
        endpoint: "api.together.ai/v1",
        provider: EndpointProvider::Known(ProviderId::TogetherAi),
        keys: KeySource::EnvUnlessGiven(&[
            "TOGETHER_API_KEY",
            "TOGETHER_AI_API_KEY",
            "TOGETHERAI_API_KEY",
            "TOGETHER_AI_TOKEN",
        ]),
    },
    Branch {
        endpoint: "api.together.xyz/v1",
        provider: EndpointProvider::Known(ProviderId::TogetherAi),
        keys: KeySource::EnvUnlessGiven(&[
            "TOGETHER_API_KEY",
            "TOGETHER_AI_API_KEY",
            "TOGETHERAI_API_KEY",
            "TOGETHER_AI_TOKEN",
        ]),
    },
    Branch {
        endpoint: "app.empower.dev/api/v1",
        provider: EndpointProvider::Known(ProviderId::Empower),
        keys: KeySource::Env(&["EMPOWER_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.friendli.ai/serverless/v1",
        provider: EndpointProvider::Known(ProviderId::Friendliai),
        keys: KeySource::Env(&["FRIENDLIAI_API_KEY", "FRIENDLI_TOKEN"]),
    },
    Branch {
        endpoint: "api.sambanova.ai/v1",
        provider: EndpointProvider::Known(ProviderId::Sambanova),
        keys: KeySource::Env(&["SAMBANOVA_API_KEY"]),
    },
    Branch {
        endpoint: "ollama.com",
        provider: EndpointProvider::Known(ProviderId::Ollama),
        keys: KeySource::Env(&["OLLAMA_API_KEY"]),
    },
    Branch {
        endpoint: "https://api-inference.modelscope.cn/v1",
        provider: EndpointProvider::Known(ProviderId::Modelscope),
        keys: KeySource::Env(&["MODELSCOPE_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.synthetic.new/openai/v1",
        provider: EndpointProvider::Known(ProviderId::Synthetic),
        keys: KeySource::Env(&["SYNTHETIC_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.stima.tech/v1",
        provider: EndpointProvider::Known(ProviderId::Apertis),
        keys: KeySource::Env(&["STIMA_API_KEY"]),
    },
    Branch {
        endpoint: "https://nano-gpt.com/api/v1",
        provider: EndpointProvider::Known(ProviderId::Nanogpt),
        keys: KeySource::Env(&["NANOGPT_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.poe.com/v1",
        provider: EndpointProvider::Known(ProviderId::Poe),
        keys: KeySource::Env(&["POE_API_KEY"]),
    },
    Branch {
        endpoint: "https://llm.chutes.ai/v1/",
        provider: EndpointProvider::Known(ProviderId::Chutes),
        keys: KeySource::Env(&["CHUTES_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.v0.dev/v1",
        provider: EndpointProvider::Known(ProviderId::V0),
        keys: KeySource::Env(&["V0_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.lambda.ai/v1",
        provider: EndpointProvider::Known(ProviderId::LambdaAi),
        keys: KeySource::Env(&["LAMBDA_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.inceptionlabs.ai/v1",
        provider: EndpointProvider::Known(ProviderId::Inception),
        keys: KeySource::Env(&["INCEPTION_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.hyperbolic.xyz/v1",
        provider: EndpointProvider::Known(ProviderId::Hyperbolic),
        keys: KeySource::Env(&["HYPERBOLIC_API_KEY"]),
    },
    Branch {
        endpoint: "https://ai-gateway.vercel.sh/v1",
        provider: EndpointProvider::Known(ProviderId::VercelAiGateway),
        keys: KeySource::Env(&["VERCEL_AI_GATEWAY_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.edenai.run/v3",
        provider: EndpointProvider::Known(ProviderId::Edenai),
        keys: KeySource::Env(&["EDENAI_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.inference.wandb.ai/v1",
        provider: EndpointProvider::Known(ProviderId::Wandb),
        keys: KeySource::Env(&["WANDB_API_KEY"]),
    },
    Branch {
        endpoint: "https://pinstripes.io/v1",
        provider: EndpointProvider::Known(ProviderId::Pinstripes),
        keys: KeySource::Env(&["PINSTRIPES_API_KEY"]),
    },
    Branch {
        endpoint: "https://api.meta.ai/v1",
        provider: EndpointProvider::Known(ProviderId::Meta),
        keys: KeySource::Env(&["META_API_KEY"]),
    },
    Branch {
        endpoint: "https://gigachat.devices.sberbank.ru/api/v1",
        provider: EndpointProvider::Known(ProviderId::Gigachat),
        keys: KeySource::Env(&["GIGACHAT_API_KEY"]),
    },
];
