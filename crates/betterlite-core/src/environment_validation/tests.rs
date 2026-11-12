#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use betterlite_models::resolution::ProviderIdentity;
use rstest::rstest;

use super::rows::{MODEL_CHAIN, provider_row};
use super::{EnvironmentCheck, EnvironmentRequest, validate_environment};
use crate::cost::context::ProviderInference;

mod config_args;
mod missing_filter;
mod model_set;
mod probe;
mod provider_fields;

struct Inference(Result<ProviderIdentity, &'static str>);

impl ProviderInference for Inference {
    type Failure = &'static str;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, &'static str> {
        self.0.clone()
    }
}

fn names(present: &[&str]) -> BTreeSet<String> {
    present.iter().map(|name| (*name).to_owned()).collect()
}

fn with_provider(provider: &str, present: &[&str]) -> EnvironmentCheck {
    let inference = Inference(Ok(ProviderIdentity::Custom(provider.to_owned())));
    let request = EnvironmentRequest {
        model: Some("any-model"),
        api_key: None,
        api_base: None,
        api_version: None,
    };
    validate_environment(
        &request,
        &names(present),
        &ProviderModelSets::default(),
        &inference,
    )
}

fn missing(missing_keys: &[&'static str]) -> EnvironmentCheck {
    EnvironmentCheck {
        keys_in_environment: missing_keys.is_empty(),
        missing_keys: missing_keys.to_vec(),
    }
}

#[rstest]
#[case::openai("openai", &["OPENAI_API_KEY"])]
#[case::azure("azure", &["AZURE_API_BASE", "AZURE_API_VERSION", "AZURE_API_KEY"])]
#[case::anthropic("anthropic", &["ANTHROPIC_API_KEY"])]
#[case::cohere("cohere", &["COHERE_API_KEY"])]
#[case::replicate("replicate", &["REPLICATE_API_KEY"])]
#[case::openrouter("openrouter", &["OPENROUTER_API_KEY"])]
#[case::vercel_ai_gateway("vercel_ai_gateway", &["VERCEL_AI_GATEWAY_API_KEY"])]
#[case::edenai("edenai", &["EDENAI_API_KEY"])]
#[case::datarobot("datarobot", &["DATAROBOT_API_TOKEN"])]
#[case::vertex_ai("vertex_ai", &["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"])]
#[case::huggingface("huggingface", &["HUGGINGFACE_API_KEY"])]
#[case::ai21("ai21", &["AI21_API_KEY"])]
#[case::together_ai("together_ai", &["TOGETHERAI_API_KEY"])]
#[case::aleph_alpha("aleph_alpha", &["ALEPH_ALPHA_API_KEY"])]
#[case::baseten("baseten", &["BASETEN_API_KEY"])]
#[case::nlp_cloud("nlp_cloud", &["NLP_CLOUD_API_KEY"])]
#[case::bedrock("bedrock", &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"])]
#[case::sagemaker("sagemaker", &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"])]
#[case::ollama("ollama", &["OLLAMA_API_BASE"])]
#[case::ollama_chat("ollama_chat", &["OLLAMA_API_BASE"])]
#[case::anyscale("anyscale", &["ANYSCALE_API_KEY"])]
#[case::deepinfra("deepinfra", &["DEEPINFRA_API_KEY"])]
#[case::featherless_ai("featherless_ai", &["FEATHERLESS_AI_API_KEY"])]
#[case::gemini("gemini", &["GOOGLE_API_KEY", "GEMINI_API_KEY"])]
#[case::groq("groq", &["GROQ_API_KEY"])]
#[case::nvidia_nim("nvidia_nim", &["NVIDIA_NIM_API_KEY"])]
#[case::cerebras("cerebras", &["CEREBRAS_API_KEY"])]
#[case::nadir("nadir", &["NADIR_API_KEY"])]
#[case::xai("xai", &["XAI_API_KEY"])]
#[case::ai21_chat("ai21_chat", &["AI21_API_KEY"])]
#[case::volcengine("volcengine", &["VOLCENGINE_API_KEY"])]
#[case::codestral("codestral", &["CODESTRAL_API_KEY"])]
#[case::text_completion_codestral("text-completion-codestral", &["CODESTRAL_API_KEY"])]
#[case::inception("inception", &["INCEPTION_API_KEY"])]
#[case::text_completion_inception("text-completion-inception", &["INCEPTION_API_KEY"])]
#[case::deepseek("deepseek", &["DEEPSEEK_API_KEY"])]
#[case::tencent("tencent", &["TENCENT_API_KEY"])]
#[case::mistral("mistral", &["MISTRAL_API_KEY"])]
#[case::palm("palm", &["PALM_API_KEY"])]
#[case::perplexity("perplexity", &["PERPLEXITYAI_API_KEY"])]
#[case::voyage("voyage", &["VOYAGE_API_KEY"])]
#[case::infinity("infinity", &["INFINITY_API_KEY"])]
#[case::fireworks_ai("fireworks_ai", &["FIREWORKS_AI_API_KEY"])]
#[case::cloudflare("cloudflare", &["CLOUDFLARE_API_KEY", "CLOUDFLARE_API_BASE"])]
#[case::novita("novita", &["NOVITA_API_KEY"])]
#[case::nebius("nebius", &["NEBIUS_API_KEY"])]
#[case::wandb("wandb", &["WANDB_API_KEY"])]
#[case::dashscope("dashscope", &["DASHSCOPE_API_KEY"])]
#[case::qwencloud("qwencloud", &["QWENCLOUD_API_KEY"])]
#[case::qwen_ai_platform("qwen_ai_platform", &["QWEN_AI_PLATFORM_API_KEY"])]
#[case::modelscope("modelscope", &["MODELSCOPE_API_KEY"])]
#[case::moonshot("moonshot", &["MOONSHOT_API_KEY"])]
fn provider_row_reports_its_names_when_absent_and_none_when_present(
    #[case] provider: &str,
    #[case] required: &[&'static str],
) {
    assert_eq!(with_provider(provider, &[]), missing(required));
    assert_eq!(with_provider(provider, required), missing(&[]));
    assert_eq!(
        with_provider(provider, &["UNRELATED_NAME"]),
        missing(required)
    );
}

#[rstest]
#[case::anthropic_key("anthropic", &["ANTHROPIC_API_KEY"])]
#[case::anthropic_auth_token("anthropic", &["ANTHROPIC_AUTH_TOKEN"])]
#[case::gemini_google("gemini", &["GOOGLE_API_KEY"])]
#[case::gemini_gemini("gemini", &["GEMINI_API_KEY"])]
#[case::fireworks_ai_key("fireworks_ai", &["FIREWORKS_AI_API_KEY"])]
#[case::fireworks_key("fireworks_ai", &["FIREWORKS_API_KEY"])]
#[case::fireworks_ai_without_underscore("fireworks_ai", &["FIREWORKSAI_API_KEY"])]
#[case::fireworks_token("fireworks_ai", &["FIREWORKS_AI_TOKEN"])]
#[case::bedrock_key_pair("bedrock", &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"])]
#[case::bedrock_role("bedrock", &["AWS_ROLE_ARN"])]
#[case::bedrock_profile("bedrock", &["AWS_PROFILE"])]
#[case::bedrock_web_identity("bedrock", &["AWS_WEB_IDENTITY_TOKEN_FILE"])]
#[case::bedrock_ecs_relative("bedrock", &["AWS_CONTAINER_CREDENTIALS_RELATIVE_URI"])]
#[case::bedrock_ecs_full("bedrock", &["AWS_CONTAINER_CREDENTIALS_FULL_URI"])]
#[case::sagemaker_profile("sagemaker", &["AWS_PROFILE"])]
#[case::sagemaker_ecs_full("sagemaker", &["AWS_CONTAINER_CREDENTIALS_FULL_URI"])]
#[case::cloudflare_account("cloudflare", &["CLOUDFLARE_API_KEY", "CLOUDFLARE_ACCOUNT_ID"])]
#[case::cloudflare_base("cloudflare", &["CLOUDFLARE_API_KEY", "CLOUDFLARE_API_BASE"])]
#[case::qwencloud_dashscope_key("qwencloud", &["DASHSCOPE_API_KEY"])]
#[case::qwen_ai_platform_dashscope_key("qwen_ai_platform", &["DASHSCOPE_API_KEY"])]
fn provider_row_accepts_one_of_several_names(#[case] provider: &str, #[case] present: &[&str]) {
    assert_eq!(with_provider(provider, present), missing(&[]));
}

#[rstest]
#[case::bedrock_access_key_alone("bedrock", &["AWS_ACCESS_KEY_ID"], &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"])]
#[case::bedrock_secret_alone("bedrock", &["AWS_SECRET_ACCESS_KEY"], &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"])]
#[case::cloudflare_key_alone("cloudflare", &["CLOUDFLARE_API_KEY"], &["CLOUDFLARE_API_KEY", "CLOUDFLARE_API_BASE"])]
#[case::cloudflare_account_alone("cloudflare", &["CLOUDFLARE_ACCOUNT_ID"], &["CLOUDFLARE_API_KEY", "CLOUDFLARE_API_BASE"])]
#[case::cloudflare_base_alone("cloudflare", &["CLOUDFLARE_API_BASE"], &["CLOUDFLARE_API_KEY", "CLOUDFLARE_API_BASE"])]
#[case::azure_partial("azure", &["AZURE_API_KEY", "AZURE_API_BASE"], &["AZURE_API_BASE", "AZURE_API_VERSION", "AZURE_API_KEY"])]
#[case::vertex_project_alone("vertex_ai", &["VERTEXAI_PROJECT"], &["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"])]
#[case::vertex_location_alone("vertex_ai", &["VERTEXAI_LOCATION"], &["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"])]
#[case::qwencloud_other_family_key("qwencloud", &["QWEN_AI_PLATFORM_API_KEY"], &["QWENCLOUD_API_KEY"])]
#[case::dashscope_ignores_qwencloud_key("dashscope", &["QWENCLOUD_API_KEY"], &["DASHSCOPE_API_KEY"])]
fn provider_row_needs_every_name_of_a_group(
    #[case] provider: &str,
    #[case] present: &[&str],
    #[case] expected_missing: &[&'static str],
) {
    assert_eq!(with_provider(provider, present), missing(expected_missing));
}

#[rstest]
#[case::empty_provider_name("")]
#[case::provider_without_a_row("groq_but_not_really")]
#[case::provider_with_a_field_list_only("databricks")]
fn provider_row_unknown_provider_gives_true_and_nothing_missing(#[case] provider: &str) {
    assert_eq!(with_provider(provider, &[]), missing(&[]));
}

#[test]
fn provider_row_known_provider_id_resolves_by_its_slug() {
    let inference = Inference(Ok(ProviderIdentity::Known(ProviderId::Anthropic)));
    let request = EnvironmentRequest {
        model: Some("claude"),
        api_key: None,
        api_base: None,
        api_version: None,
    };
    let answer = validate_environment(
        &request,
        &names(&[]),
        &ProviderModelSets::default(),
        &inference,
    );
    assert_eq!(answer, missing(&["ANTHROPIC_API_KEY"]));
}

#[test]
fn provider_row_every_chain_provider_has_a_row() {
    assert!(
        MODEL_CHAIN
            .iter()
            .all(|(_, provider)| provider_row(provider).is_some())
    );
}

#[test]
fn provider_row_a_resolved_provider_skips_the_model_set_chain() {
    let mut sets = ProviderModelSets::default();
    sets.sets
        .entry(ProviderModelSet::Anthropic)
        .or_default()
        .insert("shared-model".to_owned());
    let inference = Inference(Ok(ProviderIdentity::Custom("openai".to_owned())));
    let request = EnvironmentRequest {
        model: Some("shared-model"),
        api_key: None,
        api_base: None,
        api_version: None,
    };
    let answer = validate_environment(&request, &names(&[]), &sets, &inference);
    assert_eq!(answer, missing(&["OPENAI_API_KEY"]));
}

fn sets_with(set_models: &[(ProviderModelSet, &str)]) -> ProviderModelSets {
    let mut sets = ProviderModelSets::default();
    for (set, model) in set_models {
        sets.sets
            .entry(*set)
            .or_default()
            .insert((*model).to_owned());
    }
    sets
}

fn check_without_provider(
    model: Option<&str>,
    model_sets: &ProviderModelSets,
    present: &[&str],
) -> EnvironmentCheck {
    let request = EnvironmentRequest {
        model,
        api_key: None,
        api_base: None,
        api_version: None,
    };
    let inference = Inference(Err("no provider"));
    validate_environment(&request, &names(present), model_sets, &inference)
}
