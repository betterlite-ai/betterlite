#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
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
#[case::xai_global_key_before_env(ProviderId::Xai, "grok-beta", Inputs { api_base: None, api_key: None, env: &[("XAI_API_KEY", "env-key")] }, GlobalCredentials { xai_key: Some("global-key".to_owned()), ..GlobalCredentials::default() }, Expected::Resolved { model: "grok-beta", api_base: Some("https://api.x.ai/v1"), api_key: Some("global-key") })]
#[case::inception_default(ProviderId::Inception, "inception-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "inception-model", api_base: Some("https://api.inceptionlabs.ai/v1"), api_key: None })]
#[case::inception_explicit_base_without_key_skips_fallback(ProviderId::Inception, "inception-model", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[("INCEPTION_API_KEY", "env-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "inception-model", api_base: Some("https://custom.example"), api_key: None })]
#[case::inception_explicit_base_with_explicit_key_still_resolves(ProviderId::Inception, "inception-model", Inputs { api_base: Some("https://custom.example"), api_key: Some("explicit-key"), env: &[] }, GlobalCredentials::default(), Expected::Resolved { model: "inception-model", api_base: Some("https://custom.example"), api_key: Some("explicit-key") })]
#[case::lemonade_default_literal_key(ProviderId::Lemonade, "lemonade-model", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "lemonade-model", api_base: Some("http://localhost:8000/api/v1"), api_key: Some("lemonade") })]
#[case::lemonade_explicit_base_without_key_keeps_literal(ProviderId::Lemonade, "lemonade-model", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[("LEMONADE_API_KEY", "env-key")] }, GlobalCredentials::default(), Expected::Resolved { model: "lemonade-model", api_base: Some("https://custom.example"), api_key: Some("lemonade") })]
#[case::ragflow_rewrites_model_with_no_defaults(ProviderId::Ragflow, "chat/abc123", NO_INPUTS, GlobalCredentials::default(), Expected::Resolved { model: "ragflow/chat/abc123", api_base: None, api_key: None })]
#[case::ragflow_uses_global_credentials(ProviderId::Ragflow, "chat/abc123", NO_INPUTS, GlobalCredentials { api_base: Some("https://ragflow.example".to_owned()), api_key: Some(SecretString::from("global-key")), ..GlobalCredentials::default() }, Expected::Resolved { model: "ragflow/chat/abc123", api_base: Some("https://ragflow.example"), api_key: Some("global-key") })]
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
            result.unwrap_err();
        }
    }
}
