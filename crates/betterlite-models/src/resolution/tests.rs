#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{ProviderIdentity, ResolutionContext, ResolutionError, ResolutionRequest, resolve};
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;

const CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat"},
    "text-embedding-ada-002": {"litellm_provider": "openai", "mode": "embedding"},
    "ft:babbage-002": {"litellm_provider": "openai"},
    "ft:babbage-002:acme": {"litellm_provider": "openai"},
    "ft:babbage-002:acme:org::id": {"litellm_provider": "openai"},
    "gpt-3.5-turbo-instruct": {"litellm_provider": "text-completion-openai"},
    "claude-2": {"litellm_provider": "anthropic"},
    "claude-3-5-sonnet-20240620": {"litellm_provider": "anthropic"},
    "command": {"litellm_provider": "cohere"},
    "command-r": {"litellm_provider": "cohere_chat"},
    "meta-llama/llama-3-70b-instruct": {"litellm_provider": "openrouter"},
    "claude-3-5-sonnet@20240620": {"litellm_provider": "vertex_ai-chat-models"},
    "gemini-1.5-pro": {"litellm_provider": "vertex_ai-language-models"},
    "imagen-3": {"litellm_provider": "vertex_ai-image-models"},
    "jamba-instruct": {"litellm_provider": "ai21", "mode": "chat"},
    "j2-ultra": {"litellm_provider": "ai21", "mode": "completion"},
    "aleph-model": {"litellm_provider": "aleph_alpha"},
    "nlp-cloud-model": {"litellm_provider": "nlp_cloud"},
    "amazon.titan-text": {"litellm_provider": "bedrock"},
    "amazon.titan-guardrail": {"litellm_provider": "bedrock", "mode": "guardrail"},
    "bedrock/us-east-1/some-model": {"litellm_provider": "bedrock"},
    "amazon/month-commitment-v1": {"litellm_provider": "bedrock"},
    "bedrock-converse-model": {"litellm_provider": "bedrock_converse"},
    "watsonx-model": {"litellm_provider": "watsonx"},
    "empower/empower-functions": {"litellm_provider": "empower"},
    "gradient-model": {"litellm_provider": "gradient_ai"},
    "empower-custom": {"litellm_provider": "empower"},
    "claude-opus@x": {"litellm_provider": "vertex_ai-anthropic_models"},
    "foo/vertex_ai/imagen-4": {"litellm_provider": "vertex_ai-image-models"},
    "azure-model": {"litellm_provider": "azure"},
    "fallback_generalizations": {"rules": [
        {"name": "brand-new", "pattern": "^brand-new-", "model_info": {"litellm_provider": "anthropic"}}
    ]}
}"#;

fn test_catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap()
}

fn test_environment() -> BTreeMap<String, String> {
    [
        ("GROQ_API_KEY", "groq-key"),
        ("PERPLEXITYAI_API_KEY", "pplx-key"),
        ("NADIR_API_KEY", "nadir-key"),
        ("TOGETHER_API_KEY", "together-key"),
        ("AI21_API_KEY", "ai21-key"),
        ("PUBLICAI_API_KEY", "public-key"),
        ("MISTRAL_API_KEY", "mistral-key"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect()
}

fn test_custom_providers() -> BTreeSet<String> {
    BTreeSet::from(["my-provider".to_owned()])
}

fn context<'context>(
    catalog: &'context ModelCatalog,
    known: &'context BTreeSet<String>,
    custom: &'context BTreeSet<String>,
    environment: &'context BTreeMap<String, String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: known,
        custom_providers: custom,
        environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

fn observe(
    model: &str,
    custom_llm_provider: Option<&str>,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> String {
    let catalog = test_catalog();
    let known = BTreeSet::new();
    let custom = test_custom_providers();
    let environment = test_environment();
    let sentinel = api_key.map(SecretString::from);
    let request = ResolutionRequest {
        model,
        custom_llm_provider,
        api_base,
        api_key: sentinel.as_ref(),
    };
    match resolve(&request, &context(&catalog, &known, &custom, &environment)) {
        Ok(resolved) => format!(
            "{}|{}|{}|{}",
            resolved.model,
            resolved.provider.slug(),
            resolved.api_base.as_deref().unwrap_or("-"),
            resolved
                .api_key
                .as_ref()
                .map_or("-", ExposeSecret::expose_secret)
        ),
        Err(_) => String::from("error"),
    }
}

#[rstest]
#[case::explicit_openai_keeps_the_caller_key(
    "openai/gpt-4o",
    None,
    Some("http://h/v1"),
    Some("sk-x"),
    "gpt-4o|openai|http://h/v1|sk-x"
)]
#[case::bare_openai_name_resolves_from_the_catalog("gpt-4o", None, None, None, "gpt-4o|openai|-|-")]
#[case::a_single_colon_finetune_key_leaves_the_set("ft:babbage-002", None, None, None, "error")]
#[case::a_finetune_key_with_two_colons_stays_in_the_set(
    "ft:babbage-002:acme",
    None,
    None,
    None,
    "ft:babbage-002:acme|openai|-|-"
)]
#[case::a_finetune_key_with_more_colons_stays_openai(
    "ft:babbage-002:acme:org::id",
    None,
    None,
    None,
    "ft:babbage-002:acme:org::id|openai|-|-"
)]
#[case::a_gpt4_finetune_name_matches_by_substring(
    "ft:gpt-4o-mini:acme",
    None,
    None,
    None,
    "ft:gpt-4o-mini:acme|openai|-|-"
)]
#[case::text_completion_openai(
    "gpt-3.5-turbo-instruct",
    None,
    None,
    None,
    "gpt-3.5-turbo-instruct|text-completion-openai|-|-"
)]
#[case::anthropic_text("claude-2", None, None, None, "claude-2|anthropic_text|-|-")]
#[case::anthropic_chat(
    "claude-3-5-sonnet-20240620",
    None,
    None,
    None,
    "claude-3-5-sonnet-20240620|anthropic|-|-"
)]
#[case::cohere("command", None, None, None, "command|cohere|-|-")]
#[case::cohere_chat("command-r", None, None, None, "command-r|cohere_chat|-|-")]
#[case::cohere_prefix_becomes_cohere_chat(
    "cohere/command-r",
    None,
    None,
    None,
    "command-r|cohere_chat|-|-"
)]
#[case::openrouter_catalog_model(
    "meta-llama/llama-3-70b-instruct",
    None,
    None,
    None,
    "meta-llama/llama-3-70b-instruct|openrouter|-|-"
)]
#[case::openrouter_native_id(
    "openrouter/auto",
    Some("openrouter"),
    None,
    None,
    "openrouter/auto|openrouter|-|-"
)]
#[case::openrouter_nested_id(
    "openrouter/anthropic/claude-3.5-sonnet",
    Some("openrouter"),
    None,
    None,
    "anthropic/claude-3.5-sonnet|openrouter|-|-"
)]
#[case::maritalk("maritalk", None, None, None, "maritalk|maritalk|-|-")]
#[case::vertex_chat_model(
    "claude-3-5-sonnet@20240620",
    None,
    None,
    None,
    "claude-3-5-sonnet@20240620|vertex_ai|-|-"
)]
#[case::vertex_language_model("gemini-1.5-pro", None, None, None, "gemini-1.5-pro|vertex_ai|-|-")]
#[case::vertex_image_model("imagen-3", None, None, None, "imagen-3|vertex_ai|-|-")]
#[case::ai21_fills_its_base_and_key(
    "jamba-instruct",
    None,
    None,
    None,
    "jamba-instruct|ai21_chat|https://api.ai21.com/studio/v1|ai21-key"
)]
#[case::ai21_keeps_an_explicit_base(
    "j2-ultra",
    None,
    Some("http://explicit/v1"),
    None,
    "j2-ultra|ai21_chat|http://explicit/v1|ai21-key"
)]
#[case::aleph_alpha("aleph-model", None, None, None, "aleph-model|aleph_alpha|-|-")]
#[case::baseten_deployment_id("qvv0xeq", None, None, None, "qvv0xeq|baseten|-|-")]
#[case::nlp_cloud("nlp-cloud-model", None, None, None, "nlp-cloud-model|nlp_cloud|-|-")]
#[case::petals(
    "petals-team/StableBeluga2",
    None,
    None,
    None,
    "petals-team/StableBeluga2|petals|-|-"
)]
#[case::bedrock_model("amazon.titan-text", None, None, None, "amazon.titan-text|bedrock|-|-")]
#[case::bedrock_converse_model(
    "bedrock-converse-model",
    None,
    None,
    None,
    "bedrock-converse-model|bedrock|-|-"
)]
#[case::bedrock_converse_constant(
    "anthropic.claude-3-haiku-20240307-v1:0",
    None,
    None,
    None,
    "anthropic.claude-3-haiku-20240307-v1:0|bedrock|-|-"
)]
#[case::bedrock_embedding_constant(
    "cohere.embed-english-v3",
    None,
    None,
    None,
    "cohere.embed-english-v3|bedrock|-|-"
)]
#[case::a_bedrock_guardrail_model_is_not_inferred(
    "amazon.titan-guardrail",
    None,
    None,
    None,
    "error"
)]
#[case::a_bedrock_month_commitment_key_is_not_inferred(
    "amazon/month-commitment-v1",
    None,
    None,
    None,
    "error"
)]
#[case::watsonx("watsonx-model", None, None, None, "watsonx-model|watsonx|-|-")]
#[case::openai_embedding_constant(
    "text-embedding-ada-002",
    None,
    None,
    None,
    "text-embedding-ada-002|openai|-|-"
)]
#[case::empower_model(
    "empower/empower-functions",
    None,
    None,
    None,
    "empower-functions|empower|https://app.empower.dev/api/v1|-"
)]
#[case::gradient_ai("gradient-model", None, None, None, "gradient-model|gradient_ai|-|-")]
#[case::star_resolves_to_openai("*", None, None, None, "*|openai|-|-")]
#[case::bytez_prefix("bytez/model", None, None, None, "model|bytez|-|-")]
#[case::gdc_prefix("gdc/model", None, None, None, "model|gdc|-|-")]
#[case::lemonade_prefix(
    "lemonade/model",
    None,
    None,
    None,
    "model|lemonade|http://localhost:8000/api/v1|lemonade"
)]
#[case::heroku_prefix("heroku/model", None, None, None, "model|heroku|-|-")]
#[case::cometapi_prefix("cometapi/model", None, None, None, "model|cometapi|-|-")]
#[case::oci_prefix("oci/model", None, None, None, "model|oci|-|-")]
#[case::compactifai_prefix("compactifai/model", None, None, None, "model|compactifai|-|-")]
#[case::ovhcloud_prefix("ovhcloud/model", None, None, None, "model|ovhcloud|-|-")]
#[case::amazon_nova_prefix(
    "amazon_nova-model",
    None,
    None,
    None,
    "amazon_nova-model|amazon_nova|-|-"
)]
#[case::sap_prefix("sap/model", None, None, None, "model|sap|-|-")]
#[case::azure_cohere_goes_to_openai_unchanged(
    "azure/command-r",
    None,
    Some("http://azure/v1"),
    Some("az-key"),
    "azure/command-r|openai|http://azure/v1|-"
)]
#[case::a_registered_custom_provider(
    "my-provider/model",
    None,
    Some("http://custom/v1"),
    Some("k"),
    "model|my-provider|http://custom/v1|-"
)]
#[case::an_unregistered_custom_provider_keeps_its_declaration(
    "model",
    Some("my-own"),
    None,
    None,
    "my-own/model|my-own|-|-"
)]
#[case::a_fallback_generalization_names_the_provider(
    "brand-new-model",
    None,
    None,
    None,
    "brand-new-model|anthropic|-|-"
)]
#[case::an_unknown_model_fails("not-a-known-model", None, None, None, "error")]
#[case::an_endpoint_match_reads_the_environment_key(
    "not-a-known-model",
    None,
    Some("https://api.groq.com/openai/v1"),
    None,
    "not-a-known-model|groq|https://api.groq.com/openai/v1|groq-key"
)]
#[case::an_endpoint_match_ignores_an_explicit_key(
    "not-a-known-model",
    None,
    Some("https://api.perplexity.ai"),
    Some("given"),
    "not-a-known-model|perplexity|https://api.perplexity.ai|pplx-key"
)]
#[case::an_endpoint_prefers_an_explicit_key_when_litellm_does(
    "not-a-known-model",
    None,
    Some("https://api.together.ai/v1"),
    Some("given"),
    "not-a-known-model|together_ai|https://api.together.ai/v1|given"
)]
#[case::a_json_registered_endpoint_uses_its_environment_key(
    "not-a-known-model",
    None,
    Some("https://api.publicai.co/v1"),
    None,
    "not-a-known-model|publicai|https://api.publicai.co/v1|public-key"
)]
#[case::a_json_registered_endpoint_keeps_an_explicit_key(
    "not-a-known-model",
    None,
    Some("https://api.publicai.co/v1"),
    Some("given"),
    "not-a-known-model|publicai|https://api.publicai.co/v1|given"
)]
#[case::an_https_nadir_base_gets_the_key(
    "not-a-known-model",
    None,
    Some("https://api.getnadir.com/v1"),
    None,
    "not-a-known-model|nadir|https://api.getnadir.com/v1|nadir-key"
)]
#[case::an_http_nadir_base_gets_no_key(
    "not-a-known-model",
    None,
    Some("http://api.getnadir.com/v1"),
    None,
    "not-a-known-model|nadir|http://api.getnadir.com/v1|-"
)]
#[case::an_endpoint_inside_an_attacker_path_does_not_match(
    "not-a-known-model",
    None,
    Some("https://attacker.com/api.groq.com/openai/v1"),
    None,
    "error"
)]
#[case::a_path_that_only_string_prefixes_the_endpoint_does_not_match(
    "not-a-known-model",
    None,
    Some("https://api.groq.com/openai/v1x"),
    None,
    "error"
)]
#[case::an_endpoint_with_no_provider_branch_is_unresolved(
    "not-a-known-model",
    None,
    Some("https://api.x.ai/v1"),
    None,
    "error"
)]
#[case::the_endpoint_loop_runs_before_model_inference(
    "gpt-4o",
    None,
    Some("https://api.x.ai/v1"),
    None,
    "error"
)]
#[case::a_second_table_provider_loses_its_prefix(
    "mistral/mistral-large",
    None,
    None,
    None,
    "mistral-large|mistral|https://api.mistral.ai/v1|mistral-key"
)]
#[case::the_xai_prefix_is_stripped(
    "xai/grok-2",
    None,
    None,
    None,
    "grok-2|xai|https://api.x.ai/v1|-"
)]
#[case::a_ragflow_model_keeps_one_ragflow_prefix(
    "ragflow/chat/abc/gpt-4o",
    None,
    None,
    None,
    "ragflow/chat/abc/gpt-4o|ragflow|-|-"
)]
#[case::a_provider_in_neither_table_loses_its_prefix(
    "bedrock/anthropic.claude-v2",
    None,
    None,
    None,
    "anthropic.claude-v2|bedrock|-|-"
)]
#[case::dall_e_is_openai("dall-e-3", None, None, None, "dall-e-3|openai|-|-")]
#[case::gpt_image_is_openai("gpt-image-1", None, None, None, "gpt-image-1|openai|-|-")]
#[case::sora_is_openai("sora-2", None, None, None, "sora-2|openai|-|-")]
#[case::a_catalog_empower_model_outside_the_constants(
    "empower-custom",
    None,
    None,
    None,
    "empower-custom|empower|-|-"
)]
#[case::a_replicate_version_id_has_64_characters(
    "my-model:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    None,
    None,
    None,
    "my-model:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|replicate|-|-"
)]
#[case::a_63_character_version_id_is_not_replicate(
    "my-model:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    None,
    None,
    None,
    "error"
)]
#[case::a_65_character_version_id_is_not_replicate(
    "my-model:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    None,
    None,
    None,
    "error"
)]
#[case::a_namespaced_vertex_image_key_is_not_inferred(
    "foo/vertex_ai/imagen-4",
    None,
    None,
    None,
    "error"
)]
#[case::another_vertex_family_slug_is_not_vertex("claude-opus@x", None, None, None, "error")]
#[case::the_sambanova_branch_is_unreachable_like_litellm(
    "not-a-known-model",
    None,
    Some("https://api.sambanova.ai/v1"),
    None,
    "error"
)]
fn provider_resolution_tests_behavior(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] api_key: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        observe(model, custom_llm_provider, api_base, api_key),
        expected
    );
}

#[test]
fn a_prefix_that_is_a_known_model_name_skips_the_defaults() {
    let catalog = test_catalog();
    let known = BTreeSet::from(["openai".to_owned()]);
    let custom = test_custom_providers();
    let environment = test_environment();
    let sentinel = SecretString::from("sk-x");
    let request = ResolutionRequest {
        model: "openai/gpt-4o",
        custom_llm_provider: None,
        api_base: Some("http://h/v1"),
        api_key: Some(&sentinel),
    };
    let resolved = resolve(&request, &context(&catalog, &known, &custom, &environment)).unwrap();
    assert_eq!(resolved.model, "gpt-4o");
    assert_eq!(
        resolved.provider,
        ProviderIdentity::Known(ProviderId::Openai)
    );
    assert!(resolved.api_key.is_none());
}

#[test]
fn a_declaration_mismatching_the_model_prefix_is_reinserted() {
    assert_eq!(
        observe("gpt-4o", Some("openai"), None, None),
        "gpt-4o|openai|-|-"
    );
}

#[test]
fn debug_output_never_carries_the_key() {
    let catalog = test_catalog();
    let known = BTreeSet::new();
    let custom = test_custom_providers();
    let environment = test_environment();
    let sentinel = SecretString::from("s3cret-key-value");
    let request = ResolutionRequest {
        model: "openai/gpt-4o",
        custom_llm_provider: None,
        api_base: None,
        api_key: Some(&sentinel),
    };
    let resolved = resolve(&request, &context(&catalog, &known, &custom, &environment)).unwrap();
    let shown = format!("{resolved:?}");
    assert!(!shown.contains("s3cret-key-value"), "{shown}");
}

#[test]
fn a_missing_provider_error_names_the_model_and_no_key() {
    let catalog = test_catalog();
    let known = BTreeSet::new();
    let custom = test_custom_providers();
    let environment = BTreeMap::new();
    let request = ResolutionRequest {
        model: "no-such-model",
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    let error = resolve(&request, &context(&catalog, &known, &custom, &environment)).unwrap_err();
    assert!(matches!(error, ResolutionError::MissingProvider { .. }));
    assert!(error.to_string().contains("no-such-model"));
}

#[test]
fn the_empty_environment_yields_no_credential() {
    assert_eq!(
        observe(
            "not-a-known-model",
            None,
            Some("https://api.groq.com/openai/v1"),
            None
        ),
        "not-a-known-model|groq|https://api.groq.com/openai/v1|groq-key"
    );
    let catalog = test_catalog();
    let known = BTreeSet::new();
    let custom = test_custom_providers();
    let empty = BTreeMap::new();
    let request = ResolutionRequest {
        model: "not-a-known-model",
        custom_llm_provider: None,
        api_base: Some("https://api.groq.com/openai/v1"),
        api_key: None,
    };
    let resolved = resolve(&request, &context(&catalog, &known, &custom, &empty)).unwrap();
    assert!(resolved.api_key.is_none());
}
