use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_b::environment::EnvironmentLookup;

pub struct SimpleDefaults {
    pub base_env: &'static [&'static str],
    pub base_default: Option<&'static str>,
    pub key_env: &'static [&'static str],
    pub key_default: Option<&'static str>,
}

pub const SIMPLE_TABLE: &[(ProviderId, SimpleDefaults)] = &[
    (
        ProviderId::JinaAi,
        SimpleDefaults {
            base_env: &["JINA_AI_API_BASE"],
            base_default: Some("https://api.jina.ai/v1"),
            key_env: &["JINA_AI_API_KEY", "JINA_API_KEY", "JINA_AI_TOKEN"],
            key_default: None,
        },
    ),
    (
        ProviderId::Zai,
        SimpleDefaults {
            base_env: &["ZAI_API_BASE"],
            base_default: Some("https://api.z.ai/api/paas/v4"),
            key_env: &["ZAI_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::TogetherAi,
        SimpleDefaults {
            base_env: &["TOGETHER_AI_API_BASE"],
            base_default: Some("https://api.together.ai/v1"),
            key_env: &[
                "TOGETHER_API_KEY",
                "TOGETHER_AI_API_KEY",
                "TOGETHERAI_API_KEY",
                "TOGETHER_AI_TOKEN",
            ],
            key_default: None,
        },
    ),
    (
        ProviderId::Friendliai,
        SimpleDefaults {
            base_env: &["FRIENDLI_API_BASE"],
            base_default: Some("https://api.friendli.ai/serverless/v1"),
            key_env: &["FRIENDLIAI_API_KEY", "FRIENDLI_TOKEN"],
            key_default: None,
        },
    ),
    (
        ProviderId::Galadriel,
        SimpleDefaults {
            base_env: &["GALADRIEL_API_BASE"],
            base_default: Some("https://api.galadriel.com/v1"),
            key_env: &["GALADRIEL_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Novita,
        SimpleDefaults {
            base_env: &["NOVITA_API_BASE"],
            base_default: Some("https://api.novita.ai/v3/openai"),
            key_env: &["NOVITA_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Snowflake,
        SimpleDefaults {
            base_env: &[],
            base_default: None,
            key_env: &["SNOWFLAKE_JWT"],
            key_default: None,
        },
    ),
    (
        ProviderId::GradientAi,
        SimpleDefaults {
            base_env: &["GRADIENT_AI_AGENT_ENDPOINT"],
            base_default: Some("https://inference.do-ai.run"),
            key_env: &["GRADIENT_AI_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::FeatherlessAi,
        SimpleDefaults {
            base_env: &["FEATHERLESS_AI_API_BASE", "FEATHERLESS_API_BASE"],
            base_default: Some("https://api.featherless.ai/v1"),
            key_env: &["FEATHERLESS_AI_API_KEY", "FEATHERLESS_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Nscale,
        SimpleDefaults {
            base_env: &["NSCALE_API_BASE"],
            base_default: Some("https://inference.api.nscale.com/v1"),
            key_env: &["NSCALE_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Heroku,
        SimpleDefaults {
            base_env: &["HEROKU_API_BASE"],
            base_default: None,
            key_env: &["HEROKU_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Dashscope,
        SimpleDefaults {
            base_env: &["DASHSCOPE_API_BASE"],
            base_default: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
            key_env: &["DASHSCOPE_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Qwencloud,
        SimpleDefaults {
            base_env: &["QWENCLOUD_API_BASE"],
            base_default: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
            key_env: &["QWENCLOUD_API_KEY", "DASHSCOPE_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::QwenAiPlatform,
        SimpleDefaults {
            base_env: &["QWEN_AI_PLATFORM_API_BASE"],
            base_default: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
            key_env: &["QWEN_AI_PLATFORM_API_KEY", "DASHSCOPE_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Modelscope,
        SimpleDefaults {
            base_env: &["MODELSCOPE_API_BASE"],
            base_default: Some("https://api-inference.modelscope.cn/v1"),
            key_env: &["MODELSCOPE_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Moonshot,
        SimpleDefaults {
            base_env: &["MOONSHOT_API_BASE"],
            base_default: Some("https://api.moonshot.ai/v1"),
            key_env: &["MOONSHOT_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::DockerModelRunner,
        SimpleDefaults {
            base_env: &["DOCKER_MODEL_RUNNER_API_BASE"],
            base_default: Some("http://localhost:22088/engines/llama.cpp"),
            key_env: &["DOCKER_MODEL_RUNNER_API_KEY"],
            key_default: Some("dummy-key"),
        },
    ),
    (
        ProviderId::V0,
        SimpleDefaults {
            base_env: &["V0_API_BASE"],
            base_default: Some("https://api.v0.dev/v1"),
            key_env: &["V0_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Morph,
        SimpleDefaults {
            base_env: &["MORPH_API_BASE"],
            base_default: Some("https://api.morphllm.com/v1"),
            key_env: &["MORPH_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::LambdaAi,
        SimpleDefaults {
            base_env: &["LAMBDA_API_BASE"],
            base_default: Some("https://api.lambda.ai/v1"),
            key_env: &["LAMBDA_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Hyperbolic,
        SimpleDefaults {
            base_env: &["HYPERBOLIC_API_BASE"],
            base_default: Some("https://api.hyperbolic.xyz/v1"),
            key_env: &["HYPERBOLIC_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::VercelAiGateway,
        SimpleDefaults {
            base_env: &["VERCEL_AI_GATEWAY_API_BASE"],
            base_default: Some("https://ai-gateway.vercel.sh/v1"),
            key_env: &["VERCEL_AI_GATEWAY_API_KEY", "VERCEL_OIDC_TOKEN"],
            key_default: None,
        },
    ),
    (
        ProviderId::Edenai,
        SimpleDefaults {
            base_env: &["EDENAI_API_BASE"],
            base_default: Some("https://api.edenai.run/v3"),
            key_env: &["EDENAI_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::FalAi,
        SimpleDefaults {
            base_env: &[],
            base_default: None,
            key_env: &["FAL_AI_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Aiml,
        SimpleDefaults {
            base_env: &["AIML_API_BASE"],
            base_default: Some("https://api.aimlapi.com/v1"),
            key_env: &["AIML_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Wandb,
        SimpleDefaults {
            base_env: &["WANDB_API_BASE"],
            base_default: Some("https://api.inference.wandb.ai/v1"),
            key_env: &["WANDB_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Clarifai,
        SimpleDefaults {
            base_env: &[],
            base_default: Some("https://api.clarifai.com/v2/ext/openai/v1"),
            key_env: &["CLARIFAI_API_KEY"],
            key_default: Some(""),
        },
    ),
    (
        ProviderId::Langgraph,
        SimpleDefaults {
            base_env: &["LANGGRAPH_API_BASE"],
            base_default: Some("http://localhost:2024"),
            key_env: &["LANGGRAPH_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Manus,
        SimpleDefaults {
            base_env: &["MANUS_API_BASE"],
            base_default: Some("https://api.manus.im"),
            key_env: &["MANUS_API_KEY"],
            key_default: None,
        },
    ),
    (
        ProviderId::Gigachat,
        SimpleDefaults {
            base_env: &["GIGACHAT_API_BASE"],
            base_default: Some("https://gigachat.devices.sberbank.ru/api/v1"),
            key_env: &["GIGACHAT_API_KEY"],
            key_default: None,
        },
    ),
];

pub fn resolve_simple<E>(
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
        .or_else(|| defaults.base_env.iter().find_map(|name| env.lookup(name)))
        .or_else(|| defaults.base_default.map(ToOwned::to_owned));
    let key = api_key
        .map(ToOwned::to_owned)
        .or_else(|| defaults.key_env.iter().find_map(|name| env.lookup(name)))
        .or_else(|| defaults.key_default.map(ToOwned::to_owned));
    (base, key)
}
