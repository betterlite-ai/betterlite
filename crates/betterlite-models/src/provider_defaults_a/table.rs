use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_a::environment::{EnvironmentLookup, first_set};

pub struct SimpleDefaults {
    pub base_env: &'static [&'static str],
    pub base_default: Option<&'static str>,
    pub key_env: &'static [&'static str],
    pub key_default: Option<&'static str>,
}

pub(crate) fn simple_defaults<E>(
    defaults: &SimpleDefaults,
    api_base: Option<&str>,
    api_key: Option<&str>,
    env: &E,
) -> (Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let base = api_base
        .map(ToOwned::to_owned)
        .or_else(|| first_set(env, defaults.base_env))
        .or_else(|| defaults.base_default.map(ToOwned::to_owned));
    let key = resolve_key(api_key, defaults.key_env, defaults.key_default, env);
    (base, key)
}

pub(crate) fn resolve_key<E>(
    explicit: Option<&str>,
    key_env: &[&str],
    key_default: Option<&str>,
    env: &E,
) -> Option<String>
where
    E: EnvironmentLookup,
{
    explicit
        .map(ToOwned::to_owned)
        .or_else(|| first_set(env, key_env))
        .or_else(|| key_default.map(ToOwned::to_owned))
}

// ProviderId has no anyscale variant, so the table cannot hold it; the resolver passes this entry in.
pub(crate) const ANYSCALE: SimpleDefaults = SimpleDefaults {
    base_env: &["ANYSCALE_API_BASE"],
    base_default: Some("https://api.endpoints.anyscale.com/v1"),
    key_env: &["ANYSCALE_API_KEY"],
    key_default: None,
};

pub(crate) const SIMPLE_TABLE: &[(ProviderId, SimpleDefaults)] = &[
    (
        ProviderId::Perplexity,
        SimpleDefaults {
            base_env: &["PERPLEXITY_API_BASE"],
            base_default: Some("https://api.perplexity.ai"),
            key_env: &["PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Deepinfra,
        SimpleDefaults {
            base_env: &["DEEPINFRA_API_BASE"],
            base_default: Some("https://api.deepinfra.com/v1/openai"),
            key_env: &["DEEPINFRA_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Empower,
        SimpleDefaults {
            base_env: &["EMPOWER_API_BASE"],
            base_default: Some("https://app.empower.dev/api/v1"),
            key_env: &["EMPOWER_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Groq,
        SimpleDefaults {
            base_env: &["GROQ_API_BASE"],
            base_default: Some("https://api.groq.com/openai/v1"),
            key_env: &["GROQ_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::NvidiaNim,
        SimpleDefaults {
            base_env: &["NVIDIA_NIM_API_BASE"],
            base_default: Some("https://integrate.api.nvidia.com/v1"),
            key_env: &["NVIDIA_NIM_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::NvidiaRiva,
        SimpleDefaults {
            base_env: &["NVIDIA_RIVA_API_BASE"],
            base_default: None,
            key_env: &["NVIDIA_RIVA_API_KEY", "NVIDIA_NIM_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Soniox,
        SimpleDefaults {
            base_env: &["SONIOX_API_BASE"],
            base_default: Some("https://api.soniox.com"),
            key_env: &["SONIOX_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Cerebras,
        SimpleDefaults {
            base_env: &["CEREBRAS_API_BASE"],
            base_default: Some("https://api.cerebras.ai/v1"),
            key_env: &["CEREBRAS_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Sambanova,
        SimpleDefaults {
            base_env: &["SAMBANOVA_API_BASE"],
            base_default: Some("https://api.sambanova.ai/v1"),
            key_env: &["SAMBANOVA_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Llama,
        SimpleDefaults {
            base_env: &["LLAMA_API_BASE"],
            base_default: Some("https://api.llama.com/compat/v1"),
            key_env: &["LLAMA_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Nebius,
        SimpleDefaults {
            base_env: &["NEBIUS_API_BASE"],
            base_default: Some("https://api.studio.nebius.ai/v1"),
            key_env: &["NEBIUS_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Ollama,
        SimpleDefaults {
            base_env: &["OLLAMA_API_BASE"],
            base_default: Some("http://localhost:11434"),
            key_env: &["OLLAMA_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Volcengine,
        SimpleDefaults {
            base_env: &["VOLCENGINE_API_BASE"],
            base_default: Some("https://ark.cn-beijing.volces.com/api/v3"),
            key_env: &["VOLCENGINE_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Codestral,
        SimpleDefaults {
            base_env: &["CODESTRAL_API_BASE"],
            base_default: Some("https://codestral.mistral.ai/v1"),
            key_env: &["CODESTRAL_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Deepseek,
        SimpleDefaults {
            base_env: &["DEEPSEEK_API_BASE"],
            base_default: Some("https://api.deepseek.com/beta"),
            key_env: &["DEEPSEEK_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Tencent,
        SimpleDefaults {
            base_env: &["TENCENT_API_BASE"],
            base_default: Some("https://tokenhub-intl.tencentcloudmaas.com/v1"),
            key_env: &["TENCENT_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::FireworksAi,
        SimpleDefaults {
            base_env: &["FIREWORKS_API_BASE"],
            base_default: Some("https://api.fireworks.ai/inference/v1"),
            key_env: &[
                "FIREWORKS_API_KEY",
                "FIREWORKS_AI_API_KEY",
                "FIREWORKSAI_API_KEY",
                "FIREWORKS_AI_TOKEN",
            ],
            key_default: None,
        },
    ),
    (
        ProviderId::Github,
        SimpleDefaults {
            base_env: &["GITHUB_API_BASE"],
            base_default: Some("https://models.inference.ai.azure.com"),
            key_env: &["GITHUB_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::HostedVllm,
        SimpleDefaults {
            base_env: &["HOSTED_VLLM_API_BASE"],
            base_default: None,
            key_env: &["HOSTED_VLLM_API_KEY"],
            key_default: Some("fake-api-key"),
        },
    ),
    (
        ProviderId::LmStudio,
        SimpleDefaults {
            base_env: &["LM_STUDIO_API_BASE"],
            base_default: None,
            key_env: &["LM_STUDIO_API_KEY"],
            key_default: Some("fake-api-key"),
        },
    ),
    (
        ProviderId::Llamafile,
        SimpleDefaults {
            base_env: &["LLAMAFILE_API_BASE"],
            base_default: Some("http://127.0.0.1:8080/v1"),
            key_env: &["LLAMAFILE_API_KEY"],
            key_default: Some("fake-api-key"),
        },
    ),
    (
        ProviderId::LitellmProxy,
        SimpleDefaults {
            base_env: &["LITELLM_PROXY_API_BASE"],
            base_default: None,
            key_env: &["LITELLM_PROXY_API_KEY"],
            key_default: None,
        },
    ),
];
