use std::collections::BTreeSet;

use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};

pub(super) enum RequiredEnv {
    Single(&'static str),
    AllOf(&'static [&'static str]),
    // The row holds when every name of any one group is set; the reported names are `missing`.
    Alternatives {
        groups: &'static [&'static [&'static str]],
        missing: &'static [&'static str],
    },
}

impl RequiredEnv {
    pub(super) fn missing(&self, names: &BTreeSet<String>) -> Vec<&'static str> {
        match self {
            Self::Single(name) if names.contains(*name) => Vec::new(),
            Self::Single(name) => vec![*name],
            Self::AllOf(required) if required.iter().all(|name| names.contains(*name)) => {
                Vec::new()
            }
            Self::AllOf(required) => required.to_vec(),
            Self::Alternatives { groups, .. }
                if groups
                    .iter()
                    .any(|group| group.iter().all(|name| names.contains(*name))) =>
            {
                Vec::new()
            }
            Self::Alternatives { missing, .. } => missing.to_vec(),
        }
    }
}

const AWS: RequiredEnv = RequiredEnv::Alternatives {
    groups: &[
        &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"],
        &["AWS_ROLE_ARN"],
        &["AWS_PROFILE"],
        &["AWS_WEB_IDENTITY_TOKEN_FILE"],
        &["AWS_CONTAINER_CREDENTIALS_RELATIVE_URI"],
        &["AWS_CONTAINER_CREDENTIALS_FULL_URI"],
    ],
    missing: &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"],
};

pub(super) const PROVIDER_ROWS: &[(&[&str], RequiredEnv)] = &[
    (&["openai"], RequiredEnv::Single("OPENAI_API_KEY")),
    (
        &["azure"],
        RequiredEnv::AllOf(&["AZURE_API_BASE", "AZURE_API_VERSION", "AZURE_API_KEY"]),
    ),
    (
        &["anthropic"],
        RequiredEnv::Alternatives {
            groups: &[&["ANTHROPIC_API_KEY"], &["ANTHROPIC_AUTH_TOKEN"]],
            missing: &["ANTHROPIC_API_KEY"],
        },
    ),
    (&["cohere"], RequiredEnv::Single("COHERE_API_KEY")),
    (&["replicate"], RequiredEnv::Single("REPLICATE_API_KEY")),
    (&["openrouter"], RequiredEnv::Single("OPENROUTER_API_KEY")),
    (
        &["vercel_ai_gateway"],
        RequiredEnv::Single("VERCEL_AI_GATEWAY_API_KEY"),
    ),
    (&["edenai"], RequiredEnv::Single("EDENAI_API_KEY")),
    (&["datarobot"], RequiredEnv::Single("DATAROBOT_API_TOKEN")),
    (
        &["vertex_ai"],
        RequiredEnv::AllOf(&["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"]),
    ),
    (&["huggingface"], RequiredEnv::Single("HUGGINGFACE_API_KEY")),
    (&["ai21"], RequiredEnv::Single("AI21_API_KEY")),
    (&["together_ai"], RequiredEnv::Single("TOGETHERAI_API_KEY")),
    (&["aleph_alpha"], RequiredEnv::Single("ALEPH_ALPHA_API_KEY")),
    (&["baseten"], RequiredEnv::Single("BASETEN_API_KEY")),
    (&["nlp_cloud"], RequiredEnv::Single("NLP_CLOUD_API_KEY")),
    (&["bedrock", "sagemaker"], AWS),
    (
        &["ollama", "ollama_chat"],
        RequiredEnv::Single("OLLAMA_API_BASE"),
    ),
    (&["anyscale"], RequiredEnv::Single("ANYSCALE_API_KEY")),
    (&["deepinfra"], RequiredEnv::Single("DEEPINFRA_API_KEY")),
    (
        &["featherless_ai"],
        RequiredEnv::Single("FEATHERLESS_AI_API_KEY"),
    ),
    (
        &["gemini"],
        RequiredEnv::Alternatives {
            groups: &[&["GOOGLE_API_KEY"], &["GEMINI_API_KEY"]],
            missing: &["GOOGLE_API_KEY", "GEMINI_API_KEY"],
        },
    ),
    (&["groq"], RequiredEnv::Single("GROQ_API_KEY")),
    (&["nvidia_nim"], RequiredEnv::Single("NVIDIA_NIM_API_KEY")),
    (&["cerebras"], RequiredEnv::Single("CEREBRAS_API_KEY")),
    (&["nadir"], RequiredEnv::Single("NADIR_API_KEY")),
    (&["xai"], RequiredEnv::Single("XAI_API_KEY")),
    (&["ai21_chat"], RequiredEnv::Single("AI21_API_KEY")),
    (&["volcengine"], RequiredEnv::Single("VOLCENGINE_API_KEY")),
    (
        &["codestral", "text-completion-codestral"],
        RequiredEnv::Single("CODESTRAL_API_KEY"),
    ),
    (
        &["inception", "text-completion-inception"],
        RequiredEnv::Single("INCEPTION_API_KEY"),
    ),
    (&["deepseek"], RequiredEnv::Single("DEEPSEEK_API_KEY")),
    (&["tencent"], RequiredEnv::Single("TENCENT_API_KEY")),
    (&["mistral"], RequiredEnv::Single("MISTRAL_API_KEY")),
    (&["palm"], RequiredEnv::Single("PALM_API_KEY")),
    (&["perplexity"], RequiredEnv::Single("PERPLEXITYAI_API_KEY")),
    (&["voyage"], RequiredEnv::Single("VOYAGE_API_KEY")),
    (&["infinity"], RequiredEnv::Single("INFINITY_API_KEY")),
    (
        &["fireworks_ai"],
        RequiredEnv::Alternatives {
            groups: &[
                &["FIREWORKS_AI_API_KEY"],
                &["FIREWORKS_API_KEY"],
                &["FIREWORKSAI_API_KEY"],
                &["FIREWORKS_AI_TOKEN"],
            ],
            missing: &["FIREWORKS_AI_API_KEY"],
        },
    ),
    (
        &["cloudflare"],
        RequiredEnv::Alternatives {
            groups: &[
                &["CLOUDFLARE_API_KEY", "CLOUDFLARE_ACCOUNT_ID"],
                &["CLOUDFLARE_API_KEY", "CLOUDFLARE_API_BASE"],
            ],
            missing: &["CLOUDFLARE_API_KEY", "CLOUDFLARE_API_BASE"],
        },
    ),
    (&["novita"], RequiredEnv::Single("NOVITA_API_KEY")),
    (&["nebius"], RequiredEnv::Single("NEBIUS_API_KEY")),
    (&["wandb"], RequiredEnv::Single("WANDB_API_KEY")),
    (&["dashscope"], RequiredEnv::Single("DASHSCOPE_API_KEY")),
    (
        &["qwencloud"],
        RequiredEnv::Alternatives {
            groups: &[&["QWENCLOUD_API_KEY"], &["DASHSCOPE_API_KEY"]],
            missing: &["QWENCLOUD_API_KEY"],
        },
    ),
    (
        &["qwen_ai_platform"],
        RequiredEnv::Alternatives {
            groups: &[&["QWEN_AI_PLATFORM_API_KEY"], &["DASHSCOPE_API_KEY"]],
            missing: &["QWEN_AI_PLATFORM_API_KEY"],
        },
    ),
    (&["modelscope"], RequiredEnv::Single("MODELSCOPE_API_KEY")),
    (&["moonshot"], RequiredEnv::Single("MOONSHOT_API_KEY")),
];

pub(super) enum ModelSource {
    Set(ProviderModelSet),
    ByProvider(&'static str),
    Names(&'static [&'static str]),
    Prefix(&'static str),
}

impl ModelSource {
    fn holds(&self, model_sets: &ProviderModelSets, model: &str) -> bool {
        match self {
            Self::Set(set) => model_sets
                .sets
                .get(set)
                .is_some_and(|names| names.contains(model)),
            Self::ByProvider(provider) => model_sets
                .models_by_provider
                .get(*provider)
                .is_some_and(|names| names.contains(model)),
            Self::Names(names) => names.contains(&model),
            Self::Prefix(prefix) => model.starts_with(prefix),
        }
    }
}

// LiteLLM's if and elif ladder: the first row whose sources hold the model names the provider row.
pub(super) const MODEL_CHAIN: &[(&[ModelSource], &str)] = &[
    (
        &[
            ModelSource::Set(ProviderModelSet::OpenAiChatCompletion),
            ModelSource::Set(ProviderModelSet::OpenAiTextCompletion),
            // the embedding set of litellm/constants.py, then LiteLLM's two image generation models
            ModelSource::Names(&["text-embedding-ada-002", "dall-e-2", "dall-e-3"]),
            ModelSource::Prefix("gpt-image"),
        ],
        "openai",
    ),
    (
        &[ModelSource::Set(ProviderModelSet::Anthropic)],
        "anthropic",
    ),
    (&[ModelSource::Set(ProviderModelSet::Cohere)], "cohere"),
    (
        &[ModelSource::Set(ProviderModelSet::Replicate)],
        "replicate",
    ),
    (
        &[ModelSource::Set(ProviderModelSet::Openrouter)],
        "openrouter",
    ),
    (
        &[ModelSource::Set(ProviderModelSet::VercelAiGateway)],
        "vercel_ai_gateway",
    ),
    (&[ModelSource::Set(ProviderModelSet::Edenai)], "edenai"),
    (
        &[ModelSource::Set(ProviderModelSet::Datarobot)],
        "datarobot",
    ),
    (
        &[
            ModelSource::Set(ProviderModelSet::VertexChat),
            ModelSource::Set(ProviderModelSet::VertexText),
            ModelSource::ByProvider("vertex_ai"),
        ],
        "vertex_ai",
    ),
    (
        &[ModelSource::Set(ProviderModelSet::Huggingface)],
        "huggingface",
    ),
    (&[ModelSource::Set(ProviderModelSet::Ai21)], "ai21"),
    (
        &[ModelSource::Set(ProviderModelSet::TogetherAi)],
        "together_ai",
    ),
    (
        &[ModelSource::Set(ProviderModelSet::AlephAlpha)],
        "aleph_alpha",
    ),
    (&[ModelSource::Set(ProviderModelSet::Baseten)], "baseten"),
    (&[ModelSource::Set(ProviderModelSet::NlpCloud)], "nlp_cloud"),
    (&[ModelSource::Set(ProviderModelSet::Novita)], "novita"),
    (&[ModelSource::Set(ProviderModelSet::Nebius)], "nebius"),
    (&[ModelSource::Set(ProviderModelSet::Wandb)], "wandb"),
];

pub(super) fn provider_row(provider: &str) -> Option<&'static RequiredEnv> {
    PROVIDER_ROWS
        .iter()
        .find(|(providers, _)| providers.contains(&provider))
        .map(|(_, row)| row)
}

pub(super) fn model_chain_row(
    model_sets: &ProviderModelSets,
    model: &str,
) -> Option<&'static RequiredEnv> {
    let (_, provider) = MODEL_CHAIN
        .iter()
        .find(|(sources, _)| sources.iter().any(|source| source.holds(model_sets, model)))?;
    provider_row(provider)
}
