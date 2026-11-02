#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_b::error::ProviderDefaultsError;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::provider_defaults_b::model::ProviderDefaultsRequest;
use crate::provider_defaults_b::resolve::resolve_provider_defaults;

#[derive(Clone)]
struct Inputs {
    api_base: Option<&'static str>,
    api_key: Option<&'static str>,
    env: &'static [(&'static str, &'static str)],
}

const NO_INPUTS: Inputs = Inputs {
    api_base: None,
    api_key: None,
    env: &[],
};

enum Expected {
    Resolved {
        model: &'static str,
        api_base: Option<&'static str>,
        api_key: Option<&'static str>,
    },
    UnknownProvider,
    InvalidRagflowModel,
}

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::jina_ai_default(ProviderId::JinaAi, "jina-embeddings-v2", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "jina-embeddings-v2", api_base: Some("https://api.jina.ai/v1"), api_key: None })]
#[case::zai_default(ProviderId::Zai, "glm-4", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "glm-4", api_base: Some("https://api.z.ai/api/paas/v4"), api_key: None })]
#[case::together_ai_default(ProviderId::TogetherAi, "llama", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: None })]
#[case::together_ai_env_precedence(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHER_API_KEY", "from-env")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: Some("from-env") })]
#[case::together_ai_explicit_overrides_env(ProviderId::TogetherAi, "llama", Inputs { api_base: Some("https://custom.example/v1"), api_key: Some("explicit-key"), env: &[("TOGETHER_API_KEY", "from-env")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://custom.example/v1"), api_key: Some("explicit-key") })]
#[case::friendliai_default(ProviderId::Friendliai, "friendli-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "friendli-model", api_base: Some("https://api.friendli.ai/serverless/v1"), api_key: None })]
#[case::galadriel_default(ProviderId::Galadriel, "galadriel-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "galadriel-model", api_base: Some("https://api.galadriel.com/v1"), api_key: None })]
#[case::novita_default(ProviderId::Novita, "novita-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "novita-model", api_base: Some("https://api.novita.ai/v3/openai"), api_key: None })]
#[case::snowflake_default(ProviderId::Snowflake, "snowflake-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "snowflake-model", api_base: None, api_key: None })]
#[case::snowflake_missing_credential_stays_none(ProviderId::Snowflake, "snowflake-model", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[] }, GlobalCredentials::default(), Expected::Resolved { model: "snowflake-model", api_base: Some("https://custom.example"), api_key: None })]
#[case::gradient_ai_default(ProviderId::GradientAi, "gradient-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "gradient-model", api_base: Some("https://inference.do-ai.run"), api_key: None })]
#[case::featherless_ai_default(ProviderId::FeatherlessAi, "featherless-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "featherless-model", api_base: Some("https://api.featherless.ai/v1"), api_key: None })]
#[case::nscale_default(ProviderId::Nscale, "nscale-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "nscale-model", api_base: Some("https://inference.api.nscale.com/v1"), api_key: None })]
#[case::heroku_default(ProviderId::Heroku, "heroku-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "heroku-model", api_base: None, api_key: None })]
#[case::dashscope_default(ProviderId::Dashscope, "qwen-plus", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"), api_key: None })]
#[case::qwencloud_default(ProviderId::Qwencloud, "qwen-plus", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"), api_key: None })]
#[case::qwen_ai_platform_default(ProviderId::QwenAiPlatform, "qwen-plus", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"), api_key: None })]
#[case::qwencloud_shares_dashscope_key_env(ProviderId::Qwencloud, "qwen-plus", Inputs { api_base: None, api_key: None, env: &[("DASHSCOPE_API_KEY", "shared-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"), api_key: Some("shared-key") })]
#[case::modelscope_default(ProviderId::Modelscope, "modelscope-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "modelscope-model", api_base: Some("https://api-inference.modelscope.cn/v1"), api_key: None })]
#[case::moonshot_default(ProviderId::Moonshot, "moonshot-v1", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "moonshot-v1", api_base: Some("https://api.moonshot.ai/v1"), api_key: None })]
#[case::docker_model_runner_default(ProviderId::DockerModelRunner, "local-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "local-model", api_base: Some("http://localhost:22088/engines/llama.cpp"), api_key: Some("dummy-key") })]
#[case::v0_default(ProviderId::V0, "v0-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "v0-model", api_base: Some("https://api.v0.dev/v1"), api_key: None })]
#[case::morph_default(ProviderId::Morph, "morph-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "morph-model", api_base: Some("https://api.morphllm.com/v1"), api_key: None })]
#[case::lambda_ai_default(ProviderId::LambdaAi, "lambda-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "lambda-model", api_base: Some("https://api.lambda.ai/v1"), api_key: None })]
#[case::hyperbolic_default(ProviderId::Hyperbolic, "hyperbolic-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "hyperbolic-model", api_base: Some("https://api.hyperbolic.xyz/v1"), api_key: None })]
#[case::vercel_ai_gateway_default(ProviderId::VercelAiGateway, "vercel-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "vercel-model", api_base: Some("https://ai-gateway.vercel.sh/v1"), api_key: None })]
#[case::edenai_default(ProviderId::Edenai, "edenai-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "edenai-model", api_base: Some("https://api.edenai.run/v3"), api_key: None })]
#[case::fal_ai_leaves_api_base_untouched(ProviderId::FalAi, "fal-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "fal-model", api_base: None, api_key: None })]
#[case::aiml_default(ProviderId::Aiml, "aiml-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "aiml-model", api_base: Some("https://api.aimlapi.com/v1"), api_key: None })]
#[case::wandb_default(ProviderId::Wandb, "wandb-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "wandb-model", api_base: Some("https://api.inference.wandb.ai/v1"), api_key: None })]
#[case::clarifai_default_key_is_empty_string(ProviderId::Clarifai, "clarifai-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "clarifai-model", api_base: Some("https://api.clarifai.com/v2/ext/openai/v1"), api_key: Some("") })]
#[case::langgraph_default(ProviderId::Langgraph, "langgraph-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "langgraph-model", api_base: Some("http://localhost:2024"), api_key: None })]
#[case::manus_default(ProviderId::Manus, "manus-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "manus-model", api_base: Some("https://api.manus.im"), api_key: None })]
#[case::gigachat_default(ProviderId::Gigachat, "gigachat-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "gigachat-model", api_base: Some("https://gigachat.devices.sberbank.ru/api/v1"), api_key: None })]
#[case::mistral_default(ProviderId::Mistral, "mistral-small", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "mistral-small", api_base: Some("https://api.mistral.ai/v1"), api_key: None })]
#[case::mistral_appends_missing_v1_suffix(ProviderId::Mistral, "mistral-small", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[] }, GlobalCredentials::default(), Expected::Resolved { model: "mistral-small", api_base: Some("https://custom.example/v1"), api_key: None })]
#[case::mistral_azure_base_before_generic_key(ProviderId::Mistral, "mistral-small", Inputs { api_base: None, api_key: None, env: &[("MISTRAL_AZURE_API_BASE", "https://azure.example"), ("MISTRAL_API_KEY", "generic-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "mistral-small", api_base: Some("https://azure.example/v1"), api_key: Some("generic-key") })]
#[case::xai_default(ProviderId::Xai, "grok-beta", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "grok-beta", api_base: Some("https://api.x.ai/v1"), api_key: None })]
#[case::xai_global_key_before_env(ProviderId::Xai, "grok-beta", Inputs { api_base: None, api_key: None, env: &[("XAI_API_KEY", "env-key")] }, GlobalCredentials { xai_key: Some(SecretString::from("global-key")), ..GlobalCredentials::default() }, Expected::Resolved { model: "grok-beta", api_base: Some("https://api.x.ai/v1"), api_key: Some("global-key") })]
#[case::xai_explicit_base_still_resolves_key(ProviderId::Xai, "grok-beta", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[("XAI_API_KEY", "env-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "grok-beta", api_base: Some("https://custom.example"), api_key: Some("env-key") })]
#[case::inception_default(ProviderId::Inception, "inception-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "inception-model", api_base: Some("https://api.inceptionlabs.ai/v1"), api_key: None })]
#[case::inception_explicit_base_without_key_skips_fallback(ProviderId::Inception, "inception-model", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[("INCEPTION_API_KEY", "env-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "inception-model", api_base: Some("https://custom.example"), api_key: None })]
#[case::inception_explicit_base_with_explicit_key_still_resolves(ProviderId::Inception, "inception-model", Inputs { api_base: Some("https://custom.example"), api_key: Some("explicit-key"), env: &[] }, GlobalCredentials::default(), Expected::Resolved { model: "inception-model", api_base: Some("https://custom.example"), api_key: Some("explicit-key") })]
#[case::lemonade_default_literal_key(ProviderId::Lemonade, "lemonade-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "lemonade-model", api_base: Some("http://localhost:8000/api/v1"), api_key: Some("lemonade") })]
#[case::lemonade_explicit_base_without_key_keeps_literal(ProviderId::Lemonade, "lemonade-model", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[("LEMONADE_API_KEY", "env-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "lemonade-model", api_base: Some("https://custom.example"), api_key: Some("lemonade") })]
#[case::ragflow_rewrites_model_with_no_defaults(ProviderId::Ragflow, "chat/abc123/mymodel", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "ragflow/chat/abc123/mymodel", api_base: None, api_key: None })]
#[case::ragflow_uses_global_credentials(ProviderId::Ragflow, "chat/abc123/mymodel", NO_INPUTS, GlobalCredentials { api_base: Some("https://ragflow.example".to_owned()), api_key: Some(SecretString::from("global-key")), ..GlobalCredentials::default() }, Expected::Resolved { model: "ragflow/chat/abc123/mymodel", api_base: Some("https://ragflow.example"), api_key: Some("global-key") })]
#[case::ragflow_global_credentials_before_env(ProviderId::Ragflow, "chat/abc123/mymodel", Inputs { api_base: None, api_key: None, env: &[("RAGFLOW_API_BASE", "https://env.ragflow.example"), ("RAGFLOW_API_KEY", "env-ragflow-key")] }, GlobalCredentials { api_base: Some("https://global.ragflow.example".to_owned()), api_key: Some(SecretString::from("global-ragflow-key")), ..GlobalCredentials::default() }, Expected::Resolved { model: "ragflow/chat/abc123/mymodel", api_base: Some("https://global.ragflow.example"), api_key: Some("global-ragflow-key") })]
#[case::galadriel_env_defaults_fill_base_and_key(ProviderId::Galadriel, "galadriel-model", Inputs { api_base: None, api_key: None, env: &[("GALADRIEL_API_BASE", "https://env.galadriel.example/v1"), ("GALADRIEL_API_KEY", "env-galadriel-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "galadriel-model", api_base: Some("https://env.galadriel.example/v1"), api_key: Some("env-galadriel-key") })]
#[case::novita_env_defaults_fill_base_and_key(ProviderId::Novita, "novita-model", Inputs { api_base: None, api_key: None, env: &[("NOVITA_API_BASE", "https://env.novita.example/v1"), ("NOVITA_API_KEY", "env-novita-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "novita-model", api_base: Some("https://env.novita.example/v1"), api_key: Some("env-novita-key") })]
#[case::gradient_ai_env_defaults_fill_base_and_key(ProviderId::GradientAi, "gradient-model", Inputs { api_base: None, api_key: None, env: &[("GRADIENT_AI_AGENT_ENDPOINT", "https://env.gradient-ai.example"), ("GRADIENT_AI_API_KEY", "env-gradient-ai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "gradient-model", api_base: Some("https://env.gradient-ai.example"), api_key: Some("env-gradient-ai-key") })]
#[case::nscale_env_defaults_fill_base_and_key(ProviderId::Nscale, "nscale-model", Inputs { api_base: None, api_key: None, env: &[("NSCALE_API_BASE", "https://env.nscale.example/v1"), ("NSCALE_API_KEY", "env-nscale-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "nscale-model", api_base: Some("https://env.nscale.example/v1"), api_key: Some("env-nscale-key") })]
#[case::modelscope_env_defaults_fill_base_and_key(ProviderId::Modelscope, "modelscope-model", Inputs { api_base: None, api_key: None, env: &[("MODELSCOPE_API_BASE", "https://env.modelscope.example/v1"), ("MODELSCOPE_API_KEY", "env-modelscope-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "modelscope-model", api_base: Some("https://env.modelscope.example/v1"), api_key: Some("env-modelscope-key") })]
#[case::moonshot_env_defaults_fill_base_and_key(ProviderId::Moonshot, "moonshot-v1", Inputs { api_base: None, api_key: None, env: &[("MOONSHOT_API_BASE", "https://env.moonshot.example/v1"), ("MOONSHOT_API_KEY", "env-moonshot-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "moonshot-v1", api_base: Some("https://env.moonshot.example/v1"), api_key: Some("env-moonshot-key") })]
#[case::v0_env_defaults_fill_base_and_key(ProviderId::V0, "v0-model", Inputs { api_base: None, api_key: None, env: &[("V0_API_BASE", "https://env.v0.example/v1"), ("V0_API_KEY", "env-v0-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "v0-model", api_base: Some("https://env.v0.example/v1"), api_key: Some("env-v0-key") })]
#[case::morph_env_defaults_fill_base_and_key(ProviderId::Morph, "morph-model", Inputs { api_base: None, api_key: None, env: &[("MORPH_API_BASE", "https://env.morph.example/v1"), ("MORPH_API_KEY", "env-morph-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "morph-model", api_base: Some("https://env.morph.example/v1"), api_key: Some("env-morph-key") })]
#[case::lambda_ai_env_defaults_fill_base_and_key(ProviderId::LambdaAi, "lambda-model", Inputs { api_base: None, api_key: None, env: &[("LAMBDA_API_BASE", "https://env.lambda.example/v1"), ("LAMBDA_API_KEY", "env-lambda-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "lambda-model", api_base: Some("https://env.lambda.example/v1"), api_key: Some("env-lambda-key") })]
#[case::hyperbolic_env_defaults_fill_base_and_key(ProviderId::Hyperbolic, "hyperbolic-model", Inputs { api_base: None, api_key: None, env: &[("HYPERBOLIC_API_BASE", "https://env.hyperbolic.example/v1"), ("HYPERBOLIC_API_KEY", "env-hyperbolic-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "hyperbolic-model", api_base: Some("https://env.hyperbolic.example/v1"), api_key: Some("env-hyperbolic-key") })]
#[case::aiml_env_defaults_fill_base_and_key(ProviderId::Aiml, "aiml-model", Inputs { api_base: None, api_key: None, env: &[("AIML_API_BASE", "https://env.aiml.example/v1"), ("AIML_API_KEY", "env-aiml-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "aiml-model", api_base: Some("https://env.aiml.example/v1"), api_key: Some("env-aiml-key") })]
#[case::wandb_env_defaults_fill_base_and_key(ProviderId::Wandb, "wandb-model", Inputs { api_base: None, api_key: None, env: &[("WANDB_API_BASE", "https://env.wandb.example/v1"), ("WANDB_API_KEY", "env-wandb-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "wandb-model", api_base: Some("https://env.wandb.example/v1"), api_key: Some("env-wandb-key") })]
#[case::langgraph_env_defaults_fill_base_and_key(ProviderId::Langgraph, "langgraph-model", Inputs { api_base: None, api_key: None, env: &[("LANGGRAPH_API_BASE", "https://env.langgraph.example"), ("LANGGRAPH_API_KEY", "env-langgraph-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "langgraph-model", api_base: Some("https://env.langgraph.example"), api_key: Some("env-langgraph-key") })]
#[case::manus_env_defaults_fill_base_and_key(ProviderId::Manus, "manus-model", Inputs { api_base: None, api_key: None, env: &[("MANUS_API_BASE", "https://env.manus.example"), ("MANUS_API_KEY", "env-manus-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "manus-model", api_base: Some("https://env.manus.example"), api_key: Some("env-manus-key") })]
#[case::gigachat_env_defaults_fill_base_and_key(ProviderId::Gigachat, "gigachat-model", Inputs { api_base: None, api_key: None, env: &[("GIGACHAT_API_BASE", "https://env.gigachat.example/api/v1"), ("GIGACHAT_API_KEY", "env-gigachat-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "gigachat-model", api_base: Some("https://env.gigachat.example/api/v1"), api_key: Some("env-gigachat-key") })]
#[case::zai_env_defaults_fill_base_and_key(ProviderId::Zai, "glm-4", Inputs { api_base: None, api_key: None, env: &[("ZAI_API_BASE", "https://env.zai.example/api/paas/v4"), ("ZAI_API_KEY", "env-zai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "glm-4", api_base: Some("https://env.zai.example/api/paas/v4"), api_key: Some("env-zai-key") })]
#[case::docker_model_runner_env_overrides_default_key(ProviderId::DockerModelRunner, "local-model", Inputs { api_base: None, api_key: None, env: &[("DOCKER_MODEL_RUNNER_API_BASE", "https://env.docker-model-runner.example/v1"), ("DOCKER_MODEL_RUNNER_API_KEY", "env-docker-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "local-model", api_base: Some("https://env.docker-model-runner.example/v1"), api_key: Some("env-docker-key") })]
#[case::edenai_env_defaults_fill_base_and_key(ProviderId::Edenai, "edenai-model", Inputs { api_base: None, api_key: None, env: &[("EDENAI_API_BASE", "https://env.edenai.example/v3"), ("EDENAI_API_KEY", "env-edenai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "edenai-model", api_base: Some("https://env.edenai.example/v3"), api_key: Some("env-edenai-key") })]
#[case::clarifai_env_key_overrides_empty_default(ProviderId::Clarifai, "clarifai-model", Inputs { api_base: None, api_key: None, env: &[("CLARIFAI_API_KEY", "env-clarifai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "clarifai-model", api_base: Some("https://api.clarifai.com/v2/ext/openai/v1"), api_key: Some("env-clarifai-key") })]
#[case::mistral_azure_key_alone(ProviderId::Mistral, "mistral-small", Inputs { api_base: None, api_key: None, env: &[("MISTRAL_AZURE_API_KEY", "azure-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "mistral-small", api_base: Some("https://api.mistral.ai/v1"), api_key: Some("azure-key") })]
#[case::mistral_azure_key_before_generic_key(ProviderId::Mistral, "mistral-small", Inputs { api_base: None, api_key: None, env: &[("MISTRAL_AZURE_API_KEY", "azure-key"), ("MISTRAL_API_KEY", "generic-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "mistral-small", api_base: Some("https://api.mistral.ai/v1"), api_key: Some("azure-key") })]
#[case::jina_ai_base_and_primary_key_alone(ProviderId::JinaAi, "jina-embeddings-v2", Inputs { api_base: None, api_key: None, env: &[("JINA_AI_API_BASE", "https://env.jina-ai.example/v1"), ("JINA_AI_API_KEY", "env-jina-ai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "jina-embeddings-v2", api_base: Some("https://env.jina-ai.example/v1"), api_key: Some("env-jina-ai-key") })]
#[case::jina_api_key_alone(ProviderId::JinaAi, "jina-embeddings-v2", Inputs { api_base: None, api_key: None, env: &[("JINA_API_KEY", "env-jina-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "jina-embeddings-v2", api_base: Some("https://api.jina.ai/v1"), api_key: Some("env-jina-key") })]
#[case::jina_ai_token_alone(ProviderId::JinaAi, "jina-embeddings-v2", Inputs { api_base: None, api_key: None, env: &[("JINA_AI_TOKEN", "env-jina-token")] }, GlobalCredentials::default(), Expected::Resolved { model: "jina-embeddings-v2", api_base: Some("https://api.jina.ai/v1"), api_key: Some("env-jina-token") })]
#[case::jina_ai_api_key_before_jina_api_key(ProviderId::JinaAi, "jina-embeddings-v2", Inputs { api_base: None, api_key: None, env: &[("JINA_AI_API_KEY", "jina-ai-key"), ("JINA_API_KEY", "jina-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "jina-embeddings-v2", api_base: Some("https://api.jina.ai/v1"), api_key: Some("jina-ai-key") })]
#[case::jina_api_key_before_jina_ai_token(ProviderId::JinaAi, "jina-embeddings-v2", Inputs { api_base: None, api_key: None, env: &[("JINA_API_KEY", "jina-key"), ("JINA_AI_TOKEN", "jina-token")] }, GlobalCredentials::default(), Expected::Resolved { model: "jina-embeddings-v2", api_base: Some("https://api.jina.ai/v1"), api_key: Some("jina-key") })]
#[case::together_ai_base_env(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHER_AI_API_BASE", "https://env.together-ai.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://env.together-ai.example/v1"), api_key: None })]
#[case::together_ai_api_key_alone(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHER_AI_API_KEY", "env-together-ai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: Some("env-together-ai-key") })]
#[case::togetherai_api_key_alone(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHERAI_API_KEY", "env-togetherai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: Some("env-togetherai-key") })]
#[case::together_ai_token_alone(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHER_AI_TOKEN", "env-together-ai-token")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: Some("env-together-ai-token") })]
#[case::together_api_key_before_together_ai_api_key(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHER_API_KEY", "together-key"), ("TOGETHER_AI_API_KEY", "together-ai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: Some("together-key") })]
#[case::together_ai_api_key_before_togetherai_api_key(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHER_AI_API_KEY", "together-ai-key"), ("TOGETHERAI_API_KEY", "togetherai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: Some("together-ai-key") })]
#[case::togetherai_api_key_before_together_ai_token(ProviderId::TogetherAi, "llama", Inputs { api_base: None, api_key: None, env: &[("TOGETHERAI_API_KEY", "togetherai-key"), ("TOGETHER_AI_TOKEN", "together-ai-token")] }, GlobalCredentials::default(), Expected::Resolved { model: "llama", api_base: Some("https://api.together.ai/v1"), api_key: Some("togetherai-key") })]
#[case::friendliai_base_and_primary_key_alone(ProviderId::Friendliai, "friendli-model", Inputs { api_base: None, api_key: None, env: &[("FRIENDLI_API_BASE", "https://env.friendli.example/v1"), ("FRIENDLIAI_API_KEY", "env-friendliai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "friendli-model", api_base: Some("https://env.friendli.example/v1"), api_key: Some("env-friendliai-key") })]
#[case::friendli_token_alone(ProviderId::Friendliai, "friendli-model", Inputs { api_base: None, api_key: None, env: &[("FRIENDLI_TOKEN", "env-friendli-token")] }, GlobalCredentials::default(), Expected::Resolved { model: "friendli-model", api_base: Some("https://api.friendli.ai/serverless/v1"), api_key: Some("env-friendli-token") })]
#[case::friendliai_api_key_before_friendli_token(ProviderId::Friendliai, "friendli-model", Inputs { api_base: None, api_key: None, env: &[("FRIENDLIAI_API_KEY", "friendliai-key"), ("FRIENDLI_TOKEN", "friendli-token")] }, GlobalCredentials::default(), Expected::Resolved { model: "friendli-model", api_base: Some("https://api.friendli.ai/serverless/v1"), api_key: Some("friendliai-key") })]
#[case::featherless_ai_base_alone(ProviderId::FeatherlessAi, "featherless-model", Inputs { api_base: None, api_key: None, env: &[("FEATHERLESS_AI_API_BASE", "https://env.featherless-ai.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "featherless-model", api_base: Some("https://env.featherless-ai.example/v1"), api_key: None })]
#[case::featherless_base_alone(ProviderId::FeatherlessAi, "featherless-model", Inputs { api_base: None, api_key: None, env: &[("FEATHERLESS_API_BASE", "https://env.featherless.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "featherless-model", api_base: Some("https://env.featherless.example/v1"), api_key: None })]
#[case::featherless_ai_base_before_featherless_base(ProviderId::FeatherlessAi, "featherless-model", Inputs { api_base: None, api_key: None, env: &[("FEATHERLESS_AI_API_BASE", "featherless-ai-base"), ("FEATHERLESS_API_BASE", "featherless-base")] }, GlobalCredentials::default(), Expected::Resolved { model: "featherless-model", api_base: Some("featherless-ai-base"), api_key: None })]
#[case::featherless_ai_key_alone(ProviderId::FeatherlessAi, "featherless-model", Inputs { api_base: None, api_key: None, env: &[("FEATHERLESS_AI_API_KEY", "env-featherless-ai-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "featherless-model", api_base: Some("https://api.featherless.ai/v1"), api_key: Some("env-featherless-ai-key") })]
#[case::featherless_key_alone(ProviderId::FeatherlessAi, "featherless-model", Inputs { api_base: None, api_key: None, env: &[("FEATHERLESS_API_KEY", "env-featherless-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "featherless-model", api_base: Some("https://api.featherless.ai/v1"), api_key: Some("env-featherless-key") })]
#[case::featherless_ai_key_before_featherless_key(ProviderId::FeatherlessAi, "featherless-model", Inputs { api_base: None, api_key: None, env: &[("FEATHERLESS_AI_API_KEY", "featherless-ai-key"), ("FEATHERLESS_API_KEY", "featherless-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "featherless-model", api_base: Some("https://api.featherless.ai/v1"), api_key: Some("featherless-ai-key") })]
#[case::qwencloud_base_alone(ProviderId::Qwencloud, "qwen-plus", Inputs { api_base: None, api_key: None, env: &[("QWENCLOUD_API_BASE", "https://env.qwencloud.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://env.qwencloud.example/v1"), api_key: None })]
#[case::qwencloud_key_alone(ProviderId::Qwencloud, "qwen-plus", Inputs { api_base: None, api_key: None, env: &[("QWENCLOUD_API_KEY", "env-qwencloud-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"), api_key: Some("env-qwencloud-key") })]
#[case::qwencloud_key_before_dashscope_key(ProviderId::Qwencloud, "qwen-plus", Inputs { api_base: None, api_key: None, env: &[("QWENCLOUD_API_KEY", "qwencloud-key"), ("DASHSCOPE_API_KEY", "dashscope-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"), api_key: Some("qwencloud-key") })]
#[case::qwen_ai_platform_base_alone(ProviderId::QwenAiPlatform, "qwen-plus", Inputs { api_base: None, api_key: None, env: &[("QWEN_AI_PLATFORM_API_BASE", "https://env.qwen-ai-platform.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://env.qwen-ai-platform.example/v1"), api_key: None })]
#[case::qwen_ai_platform_key_before_dashscope_key(ProviderId::QwenAiPlatform, "qwen-plus", Inputs { api_base: None, api_key: None, env: &[("QWEN_AI_PLATFORM_API_KEY", "qwen-ai-platform-key"), ("DASHSCOPE_API_KEY", "dashscope-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "qwen-plus", api_base: Some("https://dashscope.aliyuncs.com/compatible-mode/v1"), api_key: Some("qwen-ai-platform-key") })]
#[case::vercel_ai_gateway_base_alone(ProviderId::VercelAiGateway, "vercel-model", Inputs { api_base: None, api_key: None, env: &[("VERCEL_AI_GATEWAY_API_BASE", "https://env.vercel-gateway.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "vercel-model", api_base: Some("https://env.vercel-gateway.example/v1"), api_key: None })]
#[case::vercel_ai_gateway_key_alone(ProviderId::VercelAiGateway, "vercel-model", Inputs { api_base: None, api_key: None, env: &[("VERCEL_AI_GATEWAY_API_KEY", "env-vercel-gateway-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "vercel-model", api_base: Some("https://ai-gateway.vercel.sh/v1"), api_key: Some("env-vercel-gateway-key") })]
#[case::vercel_ai_gateway_key_before_oidc_token(ProviderId::VercelAiGateway, "vercel-model", Inputs { api_base: None, api_key: None, env: &[("VERCEL_AI_GATEWAY_API_KEY", "vercel-gateway-key"), ("VERCEL_OIDC_TOKEN", "oidc-token")] }, GlobalCredentials::default(), Expected::Resolved { model: "vercel-model", api_base: Some("https://ai-gateway.vercel.sh/v1"), api_key: Some("vercel-gateway-key") })]
#[case::xai_base_env_alone(ProviderId::Xai, "grok-beta", Inputs { api_base: None, api_key: None, env: &[("XAI_API_BASE", "https://env.xai.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "grok-beta", api_base: Some("https://env.xai.example/v1"), api_key: None })]
#[case::xai_explicit_key_over_global_xai_key(ProviderId::Xai, "grok-beta", Inputs { api_base: None, api_key: Some("explicit-key"), env: &[] }, GlobalCredentials { xai_key: Some(SecretString::from("global-key")), ..GlobalCredentials::default() }, Expected::Resolved { model: "grok-beta", api_base: Some("https://api.x.ai/v1"), api_key: Some("explicit-key") })]
#[case::inception_base_env_alone(ProviderId::Inception, "inception-model", Inputs { api_base: None, api_key: None, env: &[("INCEPTION_API_BASE", "https://env.inception.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "inception-model", api_base: Some("https://env.inception.example/v1"), api_key: None })]
#[case::inception_global_key_before_env_key(ProviderId::Inception, "inception-model", Inputs { api_base: None, api_key: None, env: &[("INCEPTION_API_KEY", "env-key")] }, GlobalCredentials { inception_key: Some(SecretString::from("global-key")), ..GlobalCredentials::default() }, Expected::Resolved { model: "inception-model", api_base: Some("https://api.inceptionlabs.ai/v1"), api_key: Some("global-key") })]
#[case::lemonade_base_env_alone(ProviderId::Lemonade, "lemonade-model", Inputs { api_base: None, api_key: None, env: &[("LEMONADE_API_BASE", "https://env.lemonade.example/v1")] }, GlobalCredentials::default(), Expected::Resolved { model: "lemonade-model", api_base: Some("https://env.lemonade.example/v1"), api_key: Some("lemonade") })]
#[case::lemonade_global_key_before_env_key(ProviderId::Lemonade, "lemonade-model", Inputs { api_base: None, api_key: None, env: &[("LEMONADE_API_KEY", "env-key")] }, GlobalCredentials { lemonade_key: Some(SecretString::from("global-key")), ..GlobalCredentials::default() }, Expected::Resolved { model: "lemonade-model", api_base: Some("http://localhost:8000/api/v1"), api_key: Some("global-key") })]
#[case::ragflow_rejects_model_with_too_few_segments(
    ProviderId::Ragflow,
    "chat/abc123",
    NO_INPUTS,
    GlobalCredentials::default(),
    Expected::InvalidRagflowModel
)]
#[case::ragflow_rejects_model_with_unknown_kind(
    ProviderId::Ragflow,
    "workflow/abc123/mymodel",
    NO_INPUTS,
    GlobalCredentials::default(),
    Expected::InvalidRagflowModel
)]
#[case::unknown_provider_errors(
    ProviderId::Openai,
    "gpt-4",
    NO_INPUTS,
    GlobalCredentials::default(),
    Expected::UnknownProvider
)]
fn provider_defaults_b_behavior_tests_behavior(
    #[case] provider: ProviderId,
    #[case] model: &str,
    #[case] inputs: Inputs,
    #[case] globals: GlobalCredentials,
    #[case] expected: Expected,
) {
    let env = env_map(inputs.env);
    let request = ProviderDefaultsRequest {
        provider,
        model: model.to_owned(),
        api_base: inputs.api_base.map(str::to_owned),
        api_key: inputs.api_key.map(SecretString::from),
    };
    let result = resolve_provider_defaults(request, &globals, &env);
    match expected {
        Expected::Resolved {
            model: expected_model,
            api_base: expected_base,
            api_key: expected_key,
        } => {
            let resolved = result.unwrap();
            assert_eq!(resolved.model, expected_model);
            assert_eq!(resolved.api_base, expected_base.map(str::to_owned));
            assert_eq!(
                resolved
                    .api_key
                    .as_ref()
                    .map(|key| secrecy::ExposeSecret::expose_secret(key).to_owned()),
                expected_key.map(str::to_owned)
            );
        }
        Expected::UnknownProvider => {
            assert!(matches!(
                result.unwrap_err(),
                ProviderDefaultsError::UnknownProvider { .. }
            ));
        }
        Expected::InvalidRagflowModel => {
            assert!(matches!(
                result.unwrap_err(),
                ProviderDefaultsError::InvalidRagflowModel { .. }
            ));
        }
    }
}
