#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::provider_defaults_a::error::ProviderDefaultsError;
use crate::provider_defaults_a::model::{
    DefaultsProvider, ProviderDefaultsRequest, ResolvedProvider,
};
use crate::provider_defaults_a::policies::{MantleEndpoint, ModelSets, ProviderPolicies};
use crate::provider_defaults_a::resolve::resolve_provider_defaults;

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
        provider: DefaultsProvider,
        model: &'static str,
        api_base: Option<&'static str>,
        api_key: Option<&'static str>,
    },
    UnknownProvider,
    MissingMantleEndpoint,
    InvalidApiBase,
}

fn policies(openai_chat: &[&str], openai_text: &[&str], ai21_chat: &[&str]) -> ProviderPolicies {
    ProviderPolicies {
        models: ModelSets {
            openai_chat: openai_chat.iter().map(|name| (*name).to_owned()).collect(),
            openai_text_completion: openai_text.iter().map(|name| (*name).to_owned()).collect(),
            ai21_chat: ai21_chat.iter().map(|name| (*name).to_owned()).collect(),
        },
        mantle: None,
    }
}

fn mantle_policies(api_base: &str, api_key: Option<&str>, base_model: &str) -> ProviderPolicies {
    ProviderPolicies {
        models: ModelSets::default(),
        mantle: Some(MantleEndpoint {
            api_base: api_base.to_owned(),
            api_key: api_key.map(SecretString::from),
            base_model: base_model.to_owned(),
        }),
    }
}

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn exposed_key(resolved: &ResolvedProvider) -> Option<String> {
    resolved
        .api_key
        .as_ref()
        .map(|key| secrecy::ExposeSecret::expose_secret(key).to_owned())
}

#[rstest]
#[case::perplexity_default(DefaultsProvider::Known(ProviderId::Perplexity), "llama-3", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Perplexity), model: "llama-3", api_base: Some("https://api.perplexity.ai"), api_key: None })]
#[case::perplexity_first_key_env(DefaultsProvider::Known(ProviderId::Perplexity), "llama-3", Inputs { api_base: None, api_key: None, env: &[("PERPLEXITYAI_API_KEY", "first-key"), ("PERPLEXITY_API_KEY", "second-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Perplexity), model: "llama-3", api_base: Some("https://api.perplexity.ai"), api_key: Some("first-key") })]
#[case::perplexity_second_key_env(DefaultsProvider::Known(ProviderId::Perplexity), "llama-3", Inputs { api_base: None, api_key: None, env: &[("PERPLEXITY_API_KEY", "second-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Perplexity), model: "llama-3", api_base: Some("https://api.perplexity.ai"), api_key: Some("second-key") })]
#[case::perplexity_base_env(DefaultsProvider::Known(ProviderId::Perplexity), "llama-3", Inputs { api_base: None, api_key: None, env: &[("PERPLEXITY_API_BASE", "https://env.perplexity.example")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Perplexity), model: "llama-3", api_base: Some("https://env.perplexity.example"), api_key: None })]
#[case::anyscale_default(DefaultsProvider::Anyscale, "llama-3", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Anyscale, model: "llama-3", api_base: Some("https://api.endpoints.anyscale.com/v1"), api_key: None })]
#[case::anyscale_env_key(DefaultsProvider::Anyscale, "llama-3", Inputs { api_base: None, api_key: None, env: &[("ANYSCALE_API_KEY", "any-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Anyscale, model: "llama-3", api_base: Some("https://api.endpoints.anyscale.com/v1"), api_key: Some("any-key") })]
#[case::anyscale_base_env(DefaultsProvider::Anyscale, "llama-3", Inputs { api_base: None, api_key: None, env: &[("ANYSCALE_API_BASE", "https://env.anyscale.example/v1")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Anyscale, model: "llama-3", api_base: Some("https://env.anyscale.example/v1"), api_key: None })]
#[case::aiohttp_openai_passes_through(DefaultsProvider::Known(ProviderId::AiohttpOpenai), "x", Inputs { api_base: Some("https://base.example"), api_key: Some("key"), env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::AiohttpOpenai), model: "x", api_base: Some("https://base.example"), api_key: Some("key") })]
#[case::aiohttp_openai_ignores_environment(DefaultsProvider::Known(ProviderId::AiohttpOpenai), "x", Inputs { api_base: None, api_key: None, env: &[("AIOHTTP_OPENAI_API_KEY", "ignored")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::AiohttpOpenai), model: "x", api_base: None, api_key: None })]
#[case::deepinfra_default(DefaultsProvider::Known(ProviderId::Deepinfra), "meta-llama/Meta-Llama-3-70B", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Deepinfra), model: "meta-llama/Meta-Llama-3-70B", api_base: Some("https://api.deepinfra.com/v1/openai"), api_key: None })]
#[case::deepinfra_env(DefaultsProvider::Known(ProviderId::Deepinfra), "model", Inputs { api_base: None, api_key: None, env: &[("DEEPINFRA_API_BASE", "https://env.deepinfra.example"), ("DEEPINFRA_API_KEY", "deepinfra-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Deepinfra), model: "model", api_base: Some("https://env.deepinfra.example"), api_key: Some("deepinfra-key") })]
#[case::empower_default(DefaultsProvider::Known(ProviderId::Empower), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Empower), model: "x", api_base: Some("https://app.empower.dev/api/v1"), api_key: None })]
#[case::empower_env(DefaultsProvider::Known(ProviderId::Empower), "x", Inputs { api_base: None, api_key: None, env: &[("EMPOWER_API_BASE", "https://env.empower.example"), ("EMPOWER_API_KEY", "empower-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Empower), model: "x", api_base: Some("https://env.empower.example"), api_key: Some("empower-key") })]
#[case::groq_default(DefaultsProvider::Known(ProviderId::Groq), "llama-3", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Groq), model: "llama-3", api_base: Some("https://api.groq.com/openai/v1"), api_key: None })]
#[case::groq_explicit_overrides_environment(DefaultsProvider::Known(ProviderId::Groq), "llama-3", Inputs { api_base: Some("https://explicit.example/v1"), api_key: Some("explicit-key"), env: &[("GROQ_API_BASE", "https://env.example/v1"), ("GROQ_API_KEY", "env-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Groq), model: "llama-3", api_base: Some("https://explicit.example/v1"), api_key: Some("explicit-key") })]
#[case::groq_environment_fills_base_and_key(DefaultsProvider::Known(ProviderId::Groq), "llama-3", Inputs { api_base: None, api_key: None, env: &[("GROQ_API_BASE", "https://env.example/v1"), ("GROQ_API_KEY", "env-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Groq), model: "llama-3", api_base: Some("https://env.example/v1"), api_key: Some("env-key") })]
#[case::groq_empty_environment_counts_as_unset(DefaultsProvider::Known(ProviderId::Groq), "llama-3", Inputs { api_base: None, api_key: None, env: &[("GROQ_API_BASE", ""), ("GROQ_API_KEY", "")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Groq), model: "llama-3", api_base: Some("https://api.groq.com/openai/v1"), api_key: None })]
#[case::groq_explicit_empty_base_is_used_as_given(DefaultsProvider::Known(ProviderId::Groq), "llama-3", Inputs { api_base: Some(""), api_key: Some(""), env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Groq), model: "llama-3", api_base: Some(""), api_key: Some("") })]
#[case::nvidia_nim_default(DefaultsProvider::Known(ProviderId::NvidiaNim), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::NvidiaNim), model: "x", api_base: Some("https://integrate.api.nvidia.com/v1"), api_key: None })]
#[case::nvidia_nim_env(DefaultsProvider::Known(ProviderId::NvidiaNim), "x", Inputs { api_base: None, api_key: None, env: &[("NVIDIA_NIM_API_BASE", "https://env.nim.example"), ("NVIDIA_NIM_API_KEY", "nim-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::NvidiaNim), model: "x", api_base: Some("https://env.nim.example"), api_key: Some("nim-key") })]
#[case::nvidia_riva_has_no_default_base(DefaultsProvider::Known(ProviderId::NvidiaRiva), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::NvidiaRiva), model: "x", api_base: None, api_key: None })]
#[case::nvidia_riva_base_and_key_env(DefaultsProvider::Known(ProviderId::NvidiaRiva), "x", Inputs { api_base: None, api_key: None, env: &[("NVIDIA_RIVA_API_BASE", "grpc.nvcf.nvidia.com:443"), ("NVIDIA_RIVA_API_KEY", "riva-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::NvidiaRiva), model: "x", api_base: Some("grpc.nvcf.nvidia.com:443"), api_key: Some("riva-key") })]
#[case::nvidia_riva_falls_back_to_nim_key(DefaultsProvider::Known(ProviderId::NvidiaRiva), "x", Inputs { api_base: None, api_key: None, env: &[("NVIDIA_NIM_API_KEY", "nim-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::NvidiaRiva), model: "x", api_base: None, api_key: Some("nim-key") })]
#[case::nvidia_riva_key_env_before_nim_key(DefaultsProvider::Known(ProviderId::NvidiaRiva), "x", Inputs { api_base: None, api_key: None, env: &[("NVIDIA_NIM_API_KEY", "nim-key"), ("NVIDIA_RIVA_API_KEY", "riva-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::NvidiaRiva), model: "x", api_base: None, api_key: Some("riva-key") })]
#[case::soniox_default(DefaultsProvider::Known(ProviderId::Soniox), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Soniox), model: "x", api_base: Some("https://api.soniox.com"), api_key: None })]
#[case::soniox_env(DefaultsProvider::Known(ProviderId::Soniox), "x", Inputs { api_base: None, api_key: None, env: &[("SONIOX_API_BASE", "https://env.soniox.example"), ("SONIOX_API_KEY", "soniox-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Soniox), model: "x", api_base: Some("https://env.soniox.example"), api_key: Some("soniox-key") })]
#[case::cerebras_default(DefaultsProvider::Known(ProviderId::Cerebras), "llama3.1-8b", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Cerebras), model: "llama3.1-8b", api_base: Some("https://api.cerebras.ai/v1"), api_key: None })]
#[case::cerebras_env(DefaultsProvider::Known(ProviderId::Cerebras), "llama3.1-8b", Inputs { api_base: None, api_key: None, env: &[("CEREBRAS_API_BASE", "https://env.cerebras.example/v1"), ("CEREBRAS_API_KEY", "cerebras-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Cerebras), model: "llama3.1-8b", api_base: Some("https://env.cerebras.example/v1"), api_key: Some("cerebras-key") })]
#[case::nadir_default(DefaultsProvider::Known(ProviderId::Nadir), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nadir), model: "x", api_base: Some("https://api.getnadir.com/v1"), api_key: None })]
#[case::nadir_env_key_used_for_its_own_base(DefaultsProvider::Known(ProviderId::Nadir), "x", Inputs { api_base: None, api_key: None, env: &[("NADIR_API_KEY", "nadir-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nadir), model: "x", api_base: Some("https://api.getnadir.com/v1"), api_key: Some("nadir-key") })]
#[case::nadir_custom_base_skips_env_key(DefaultsProvider::Known(ProviderId::Nadir), "x", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[("NADIR_API_KEY", "nadir-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nadir), model: "x", api_base: Some("https://custom.example"), api_key: None })]
#[case::nadir_own_base_with_trailing_slash_keeps_env_key(DefaultsProvider::Known(ProviderId::Nadir), "x", Inputs { api_base: Some("https://api.getnadir.com/v1/"), api_key: None, env: &[("NADIR_API_KEY", "nadir-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nadir), model: "x", api_base: Some("https://api.getnadir.com/v1/"), api_key: Some("nadir-key") })]
#[case::nadir_env_base_without_caller_base_keeps_env_key(DefaultsProvider::Known(ProviderId::Nadir), "x", Inputs { api_base: None, api_key: None, env: &[("NADIR_API_BASE", "https://env.nadir.example/v1"), ("NADIR_API_KEY", "nadir-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nadir), model: "x", api_base: Some("https://env.nadir.example/v1"), api_key: Some("nadir-key") })]
#[case::nadir_custom_base_different_from_env_base_skips_env_key(DefaultsProvider::Known(ProviderId::Nadir), "x", Inputs { api_base: Some("https://caller.example/v1"), api_key: None, env: &[("NADIR_API_BASE", "https://env.nadir.example/v1"), ("NADIR_API_KEY", "nadir-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nadir), model: "x", api_base: Some("https://caller.example/v1"), api_key: None })]
#[case::baseten_dedicated_deployment(DefaultsProvider::Known(ProviderId::Baseten), "abc12345", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Baseten), model: "abc12345", api_base: Some("https://model-abc12345.api.baseten.co/environments/production/sync/v1"), api_key: None })]
#[case::baseten_shared_model(DefaultsProvider::Known(ProviderId::Baseten), "notdedicated", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Baseten), model: "notdedicated", api_base: Some("https://inference.baseten.co/v1"), api_key: None })]
#[case::baseten_strips_every_baseten_prefix(DefaultsProvider::Known(ProviderId::Baseten), "baseten/abc12345", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Baseten), model: "baseten/abc12345", api_base: Some("https://model-abc12345.api.baseten.co/environments/production/sync/v1"), api_key: None })]
#[case::baseten_explicit_base_and_env_key(DefaultsProvider::Known(ProviderId::Baseten), "abc12345", Inputs { api_base: Some("https://custom.example"), api_key: None, env: &[("BASETEN_API_KEY", "baseten-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Baseten), model: "abc12345", api_base: Some("https://custom.example"), api_key: Some("baseten-key") })]
#[case::sambanova_default(DefaultsProvider::Known(ProviderId::Sambanova), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Sambanova), model: "x", api_base: Some("https://api.sambanova.ai/v1"), api_key: None })]
#[case::sambanova_env(DefaultsProvider::Known(ProviderId::Sambanova), "x", Inputs { api_base: None, api_key: None, env: &[("SAMBANOVA_API_BASE", "https://env.sambanova.example/v1"), ("SAMBANOVA_API_KEY", "sambanova-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Sambanova), model: "x", api_base: Some("https://env.sambanova.example/v1"), api_key: Some("sambanova-key") })]
#[case::meta_llama_default(DefaultsProvider::Known(ProviderId::Llama), "Llama-3", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Llama), model: "Llama-3", api_base: Some("https://api.llama.com/compat/v1"), api_key: None })]
#[case::meta_llama_env(DefaultsProvider::Known(ProviderId::Llama), "Llama-3", Inputs { api_base: None, api_key: None, env: &[("LLAMA_API_BASE", "https://env.llama.example/v1"), ("LLAMA_API_KEY", "llama-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Llama), model: "Llama-3", api_base: Some("https://env.llama.example/v1"), api_key: Some("llama-key") })]
#[case::nebius_default(DefaultsProvider::Known(ProviderId::Nebius), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nebius), model: "x", api_base: Some("https://api.studio.nebius.ai/v1"), api_key: None })]
#[case::nebius_env(DefaultsProvider::Known(ProviderId::Nebius), "x", Inputs { api_base: None, api_key: None, env: &[("NEBIUS_API_BASE", "https://env.nebius.example/v1"), ("NEBIUS_API_KEY", "nebius-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Nebius), model: "x", api_base: Some("https://env.nebius.example/v1"), api_key: Some("nebius-key") })]
#[case::ollama_default(DefaultsProvider::Known(ProviderId::Ollama), "llama3", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Ollama), model: "llama3", api_base: Some("http://localhost:11434"), api_key: None })]
#[case::ollama_env(DefaultsProvider::Known(ProviderId::Ollama), "llama3", Inputs { api_base: None, api_key: None, env: &[("OLLAMA_API_BASE", "http://env.ollama.example:11434"), ("OLLAMA_API_KEY", "ollama-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Ollama), model: "llama3", api_base: Some("http://env.ollama.example:11434"), api_key: Some("ollama-key") })]
#[case::ai21_model_outside_chat_set_is_passed_through(DefaultsProvider::Known(ProviderId::Ai21), "jamba-instruct", Inputs { api_base: None, api_key: None, env: &[("AI21_API_BASE", "https://env.ai21.example"), ("AI21_API_KEY", "ai21-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Ai21), model: "jamba-instruct", api_base: None, api_key: None })]
#[case::ai21_model_in_chat_set_rewrites_provider(DefaultsProvider::Known(ProviderId::Ai21), "jamba-1.5", Inputs { api_base: None, api_key: None, env: &[("AI21_API_KEY", "ai21-key")] }, policies(&[], &[], &["jamba-1.5"]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Ai21Chat), model: "jamba-1.5", api_base: Some("https://api.ai21.com/studio/v1"), api_key: Some("ai21-key") })]
#[case::ai21_chat_provider_always_resolves(DefaultsProvider::Known(ProviderId::Ai21Chat), "jamba-instruct", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Ai21Chat), model: "jamba-instruct", api_base: Some("https://api.ai21.com/studio/v1"), api_key: None })]
#[case::ai21_chat_explicit_base_and_key(DefaultsProvider::Known(ProviderId::Ai21Chat), "jamba-1.5", Inputs { api_base: Some("https://custom.example"), api_key: Some("explicit-key"), env: &[("AI21_API_KEY", "env-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Ai21Chat), model: "jamba-1.5", api_base: Some("https://custom.example"), api_key: Some("explicit-key") })]
#[case::volcengine_default(DefaultsProvider::Known(ProviderId::Volcengine), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Volcengine), model: "x", api_base: Some("https://ark.cn-beijing.volces.com/api/v3"), api_key: None })]
#[case::volcengine_env(DefaultsProvider::Known(ProviderId::Volcengine), "x", Inputs { api_base: None, api_key: None, env: &[("VOLCENGINE_API_BASE", "https://env.volcengine.example"), ("VOLCENGINE_API_KEY", "volcengine-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Volcengine), model: "x", api_base: Some("https://env.volcengine.example"), api_key: Some("volcengine-key") })]
#[case::codestral_default(DefaultsProvider::Known(ProviderId::Codestral), "codestral-latest", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Codestral), model: "codestral-latest", api_base: Some("https://codestral.mistral.ai/v1"), api_key: None })]
#[case::codestral_env(DefaultsProvider::Known(ProviderId::Codestral), "codestral-latest", Inputs { api_base: None, api_key: None, env: &[("CODESTRAL_API_BASE", "https://env.codestral.example/v1"), ("CODESTRAL_API_KEY", "codestral-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Codestral), model: "codestral-latest", api_base: Some("https://env.codestral.example/v1"), api_key: Some("codestral-key") })]
#[case::hosted_vllm_uses_fake_key(DefaultsProvider::Known(ProviderId::HostedVllm), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::HostedVllm), model: "x", api_base: None, api_key: Some("fake-api-key") })]
#[case::hosted_vllm_env_base_and_key(DefaultsProvider::Known(ProviderId::HostedVllm), "x", Inputs { api_base: None, api_key: None, env: &[("HOSTED_VLLM_API_BASE", "https://env.vllm.example/v1"), ("HOSTED_VLLM_API_KEY", "vllm-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::HostedVllm), model: "x", api_base: Some("https://env.vllm.example/v1"), api_key: Some("vllm-key") })]
#[case::llamafile_default(DefaultsProvider::Known(ProviderId::Llamafile), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Llamafile), model: "x", api_base: Some("http://127.0.0.1:8080/v1"), api_key: Some("fake-api-key") })]
#[case::llamafile_env(DefaultsProvider::Known(ProviderId::Llamafile), "x", Inputs { api_base: None, api_key: None, env: &[("LLAMAFILE_API_BASE", "http://env.llamafile.example/v1"), ("LLAMAFILE_API_KEY", "llamafile-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Llamafile), model: "x", api_base: Some("http://env.llamafile.example/v1"), api_key: Some("llamafile-key") })]
#[case::datarobot_default_transforms_base(DefaultsProvider::Known(ProviderId::Datarobot), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Datarobot), model: "x", api_base: Some("https://app.datarobot.com/api/v2/genai/llmgw/chat/completions/"), api_key: Some("fake-api-key") })]
#[case::datarobot_standard_endpoint_adds_llmgw(DefaultsProvider::Known(ProviderId::Datarobot), "x", Inputs { api_base: Some("https://tenant.datarobot.com/api/v2"), api_key: None, env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Datarobot), model: "x", api_base: Some("https://tenant.datarobot.com/api/v2/genai/llmgw/chat/completions/"), api_key: Some("fake-api-key") })]
#[case::datarobot_deployment_path_is_kept(DefaultsProvider::Known(ProviderId::Datarobot), "x", Inputs { api_base: Some("https://tenant.datarobot.com/api/v2/deployments/abc"), api_key: None, env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Datarobot), model: "x", api_base: Some("https://tenant.datarobot.com/api/v2/deployments/abc/"), api_key: Some("fake-api-key") })]
#[case::datarobot_keeps_query_and_fragment(DefaultsProvider::Known(ProviderId::Datarobot), "x", Inputs { api_base: Some("https://tenant.datarobot.com/api/v2?x=1#frag"), api_key: None, env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Datarobot), model: "x", api_base: Some("https://tenant.datarobot.com/api/v2/genai/llmgw/chat/completions/?x=1#frag"), api_key: Some("fake-api-key") })]
#[case::datarobot_env_endpoint_and_token(DefaultsProvider::Known(ProviderId::Datarobot), "x", Inputs { api_base: None, api_key: None, env: &[("DATAROBOT_ENDPOINT", "https://tenant.datarobot.com"), ("DATAROBOT_API_TOKEN", "token")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Datarobot), model: "x", api_base: Some("https://tenant.datarobot.com/api/v2/genai/llmgw/chat/completions/"), api_key: Some("token") })]
#[case::datarobot_rejects_a_malformed_url(DefaultsProvider::Known(ProviderId::Datarobot), "x", Inputs { api_base: Some("https://[::1/api"), api_key: None, env: &[] }, policies(&[], &[], &[]), Expected::InvalidApiBase)]
#[case::lm_studio_uses_fake_key(DefaultsProvider::Known(ProviderId::LmStudio), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::LmStudio), model: "x", api_base: None, api_key: Some("fake-api-key") })]
#[case::deepseek_default(DefaultsProvider::Known(ProviderId::Deepseek), "deepseek-chat", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Deepseek), model: "deepseek-chat", api_base: Some("https://api.deepseek.com/beta"), api_key: None })]
#[case::deepseek_env(DefaultsProvider::Known(ProviderId::Deepseek), "deepseek-chat", Inputs { api_base: None, api_key: None, env: &[("DEEPSEEK_API_BASE", "https://env.deepseek.example"), ("DEEPSEEK_API_KEY", "deepseek-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Deepseek), model: "deepseek-chat", api_base: Some("https://env.deepseek.example"), api_key: Some("deepseek-key") })]
#[case::tencent_default(DefaultsProvider::Known(ProviderId::Tencent), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Tencent), model: "x", api_base: Some("https://tokenhub-intl.tencentcloudmaas.com/v1"), api_key: None })]
#[case::tencent_env(DefaultsProvider::Known(ProviderId::Tencent), "x", Inputs { api_base: None, api_key: None, env: &[("TENCENT_API_BASE", "https://env.tencent.example"), ("TENCENT_API_KEY", "tencent-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Tencent), model: "x", api_base: Some("https://env.tencent.example"), api_key: Some("tencent-key") })]
#[case::fireworks_ai_default(DefaultsProvider::Known(ProviderId::FireworksAi), "accounts/fireworks/models/llama", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::FireworksAi), model: "accounts/fireworks/models/llama", api_base: Some("https://api.fireworks.ai/inference/v1"), api_key: None })]
#[case::fireworks_ai_first_key_env(DefaultsProvider::Known(ProviderId::FireworksAi), "x", Inputs { api_base: None, api_key: None, env: &[("FIREWORKS_API_KEY", "first-key"), ("FIREWORKS_AI_API_KEY", "second-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::FireworksAi), model: "x", api_base: Some("https://api.fireworks.ai/inference/v1"), api_key: Some("first-key") })]
#[case::fireworks_ai_token_env(DefaultsProvider::Known(ProviderId::FireworksAi), "x", Inputs { api_base: None, api_key: None, env: &[("FIREWORKS_AI_TOKEN", "fw-token")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::FireworksAi), model: "x", api_base: Some("https://api.fireworks.ai/inference/v1"), api_key: Some("fw-token") })]
#[case::azure_ai_rewrites_an_openai_model(DefaultsProvider::Known(ProviderId::AzureAi), "gpt-4o", Inputs { api_base: Some("https://my.openai.azure.com"), api_key: Some("azure-key"), env: &[] }, policies(&["gpt-4o"], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Azure), model: "gpt-4o", api_base: Some("https://my.openai.azure.com"), api_key: Some("azure-key") })]
#[case::azure_ai_foundry_inference_host_keeps_provider(DefaultsProvider::Known(ProviderId::AzureAi), "gpt-4o", Inputs { api_base: Some("https://my.services.ai.azure.com"), api_key: None, env: &[] }, policies(&["gpt-4o"], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::AzureAi), model: "gpt-4o", api_base: Some("https://my.services.ai.azure.com"), api_key: None })]
#[case::azure_ai_foundry_deployments_path_rewrites(DefaultsProvider::Known(ProviderId::AzureAi), "gpt-4o", Inputs { api_base: Some("https://my.services.ai.azure.com/openai/deployments/x"), api_key: None, env: &[] }, policies(&["gpt-4o"], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Azure), model: "gpt-4o", api_base: Some("https://my.services.ai.azure.com/openai/deployments/x"), api_key: None })]
#[case::azure_ai_without_base_keeps_provider(DefaultsProvider::Known(ProviderId::AzureAi), "gpt-4o", NO_INPUTS, policies(&["gpt-4o"], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::AzureAi), model: "gpt-4o", api_base: None, api_key: None })]
#[case::azure_ai_strips_one_model_segment(DefaultsProvider::Known(ProviderId::AzureAi), "openai/gpt-4o", Inputs { api_base: Some("https://my.openai.azure.com"), api_key: None, env: &[] }, policies(&["gpt-4o"], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Azure), model: "openai/gpt-4o", api_base: Some("https://my.openai.azure.com"), api_key: None })]
#[case::azure_ai_embedding_constant_rewrites(DefaultsProvider::Known(ProviderId::AzureAi), "text-embedding-ada-002", Inputs { api_base: Some("https://my.openai.azure.com"), api_key: None, env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Azure), model: "text-embedding-ada-002", api_base: Some("https://my.openai.azure.com"), api_key: None })]
#[case::azure_ai_text_completion_model_rewrites(DefaultsProvider::Known(ProviderId::AzureAi), "gpt-3.5-turbo-instruct", Inputs { api_base: Some("https://my.openai.azure.com"), api_key: None, env: &[] }, policies(&[], &["gpt-3.5-turbo-instruct"], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Azure), model: "gpt-3.5-turbo-instruct", api_base: Some("https://my.openai.azure.com"), api_key: None })]
#[case::azure_ai_model_outside_the_sets_keeps_provider(DefaultsProvider::Known(ProviderId::AzureAi), "my-model", Inputs { api_base: Some("https://my.openai.azure.com"), api_key: None, env: &[] }, policies(&["gpt-4o"], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::AzureAi), model: "my-model", api_base: Some("https://my.openai.azure.com"), api_key: None })]
#[case::azure_ai_env_base_and_key(DefaultsProvider::Known(ProviderId::AzureAi), "gpt-4o", Inputs { api_base: None, api_key: None, env: &[("AZURE_AI_API_BASE", "https://my.openai.azure.com"), ("AZURE_AI_API_KEY", "azure-key")] }, policies(&["gpt-4o"], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Azure), model: "gpt-4o", api_base: Some("https://my.openai.azure.com"), api_key: Some("azure-key") })]
#[case::azure_ai_rejects_a_malformed_url(DefaultsProvider::Known(ProviderId::AzureAi), "gpt-4o", Inputs { api_base: Some("https://[abc]/v1"), api_key: None, env: &[] }, policies(&["gpt-4o"], &[], &[]), Expected::InvalidApiBase)]
#[case::github_default(DefaultsProvider::Known(ProviderId::Github), "gpt-4o", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Github), model: "gpt-4o", api_base: Some("https://models.inference.ai.azure.com"), api_key: None })]
#[case::github_env(DefaultsProvider::Known(ProviderId::Github), "gpt-4o", Inputs { api_base: None, api_key: None, env: &[("GITHUB_API_BASE", "https://env.github.example"), ("GITHUB_API_KEY", "github-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Github), model: "gpt-4o", api_base: Some("https://env.github.example"), api_key: Some("github-key") })]
#[case::litellm_proxy_has_no_defaults(DefaultsProvider::Known(ProviderId::LitellmProxy), "gpt-4o", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::LitellmProxy), model: "gpt-4o", api_base: None, api_key: None })]
#[case::litellm_proxy_env(DefaultsProvider::Known(ProviderId::LitellmProxy), "gpt-4o", Inputs { api_base: None, api_key: None, env: &[("LITELLM_PROXY_API_BASE", "https://proxy.example"), ("LITELLM_PROXY_API_KEY", "proxy-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::LitellmProxy), model: "gpt-4o", api_base: Some("https://proxy.example"), api_key: Some("proxy-key") })]
#[case::bedrock_mantle_uses_the_endpoint_policy(DefaultsProvider::Known(ProviderId::BedrockMantle), "us-east-1/openai.gpt-oss-120b", Inputs { api_base: None, api_key: None, env: &[] }, mantle_policies("https://bedrock-mantle.us-east-1.api.aws/v1", Some("mantle-key"), "openai.gpt-oss-120b"), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::BedrockMantle), model: "openai.gpt-oss-120b", api_base: Some("https://bedrock-mantle.us-east-1.api.aws/v1"), api_key: Some("mantle-key") })]
#[case::bedrock_mantle_without_policy_errors(DefaultsProvider::Known(ProviderId::BedrockMantle), "openai.gpt-oss-120b", NO_INPUTS, policies(&[], &[], &[]), Expected::MissingMantleEndpoint)]
#[case::json_registry_default(DefaultsProvider::Known(ProviderId::Publicai), "x", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Publicai), model: "x", api_base: Some("https://api.publicai.co/v1"), api_key: None })]
#[case::json_registry_environment(DefaultsProvider::Known(ProviderId::Publicai), "x", Inputs { api_base: None, api_key: None, env: &[("PUBLICAI_API_BASE", "https://env.publicai.example/v1"), ("PUBLICAI_API_KEY", "public-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Publicai), model: "x", api_base: Some("https://env.publicai.example/v1"), api_key: Some("public-key") })]
#[case::json_registry_explicit_overrides_environment(DefaultsProvider::Known(ProviderId::Publicai), "x", Inputs { api_base: Some("https://explicit.example/v1"), api_key: Some("explicit-key"), env: &[("PUBLICAI_API_BASE", "https://env.publicai.example/v1"), ("PUBLICAI_API_KEY", "public-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Publicai), model: "x", api_base: Some("https://explicit.example/v1"), api_key: Some("explicit-key") })]
#[case::datarobot_root_path_appends_llmgw(DefaultsProvider::Known(ProviderId::Datarobot), "x", Inputs { api_base: Some("https://tenant.datarobot.com/"), api_key: None, env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Datarobot), model: "x", api_base: Some("https://tenant.datarobot.com/api/v2/genai/llmgw/chat/completions/"), api_key: Some("fake-api-key") })]
#[case::datarobot_keeps_an_existing_llmgw_path(DefaultsProvider::Known(ProviderId::Datarobot), "x", Inputs { api_base: Some("https://tenant.datarobot.com/api/v2/genai/llmgw/chat/completions"), api_key: None, env: &[] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Datarobot), model: "x", api_base: Some("https://tenant.datarobot.com/api/v2/genai/llmgw/chat/completions/"), api_key: Some("fake-api-key") })]
#[case::baseten_model_with_eight_characters_and_a_dash(DefaultsProvider::Known(ProviderId::Baseten), "abc-1234", NO_INPUTS, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Baseten), model: "abc-1234", api_base: Some("https://inference.baseten.co/v1"), api_key: None })]
#[case::fireworks_ai_base_env(DefaultsProvider::Known(ProviderId::FireworksAi), "x", Inputs { api_base: None, api_key: None, env: &[("FIREWORKS_API_BASE", "https://my.fireworks.example/v1")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::FireworksAi), model: "x", api_base: Some("https://my.fireworks.example/v1"), api_key: None })]
#[case::fireworks_ai_second_key_env(DefaultsProvider::Known(ProviderId::FireworksAi), "x", Inputs { api_base: None, api_key: None, env: &[("FIREWORKS_AI_API_KEY", "second-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::FireworksAi), model: "x", api_base: Some("https://api.fireworks.ai/inference/v1"), api_key: Some("second-key") })]
#[case::fireworks_ai_third_key_env(DefaultsProvider::Known(ProviderId::FireworksAi), "x", Inputs { api_base: None, api_key: None, env: &[("FIREWORKSAI_API_KEY", "third-key"), ("FIREWORKS_AI_TOKEN", "fourth-token")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::FireworksAi), model: "x", api_base: Some("https://api.fireworks.ai/inference/v1"), api_key: Some("third-key") })]
#[case::lm_studio_env_base_and_key(DefaultsProvider::Known(ProviderId::LmStudio), "x", Inputs { api_base: None, api_key: None, env: &[("LM_STUDIO_API_BASE", "https://my.lmstudio.example/v1"), ("LM_STUDIO_API_KEY", "lm-key")] }, policies(&[], &[], &[]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::LmStudio), model: "x", api_base: Some("https://my.lmstudio.example/v1"), api_key: Some("lm-key") })]
#[case::ai21_model_in_chat_set_uses_env_base(DefaultsProvider::Known(ProviderId::Ai21), "jamba-1.5", Inputs { api_base: None, api_key: None, env: &[("AI21_API_BASE", "https://env.ai21.example"), ("AI21_API_KEY", "ai21-key")] }, policies(&[], &[], &["jamba-1.5"]), Expected::Resolved { provider: DefaultsProvider::Known(ProviderId::Ai21Chat), model: "jamba-1.5", api_base: Some("https://env.ai21.example"), api_key: Some("ai21-key") })]
#[case::unknown_provider_errors(DefaultsProvider::Known(ProviderId::Openai), "gpt-4o", NO_INPUTS, policies(&[], &[], &[]), Expected::UnknownProvider)]
fn provider_defaults_a_behavior_tests_behavior(
    #[case] provider: DefaultsProvider,
    #[case] model: &str,
    #[case] inputs: Inputs,
    #[case] policies: ProviderPolicies,
    #[case] expected: Expected,
) {
    let env = env_map(inputs.env);
    let registry = JSONProviderRegistry::load();
    let request = ProviderDefaultsRequest {
        provider,
        model: model.to_owned(),
        api_base: inputs.api_base.map(str::to_owned),
        api_key: inputs.api_key.map(SecretString::from),
    };
    let result = resolve_provider_defaults(request, &env, &registry, &policies);
    match expected {
        Expected::Resolved {
            provider: expected_provider,
            model: expected_model,
            api_base: expected_base,
            api_key: expected_key,
        } => {
            let resolved = result.unwrap();
            assert_eq!(resolved.provider, expected_provider);
            assert_eq!(resolved.model, expected_model);
            assert_eq!(resolved.api_base, expected_base.map(str::to_owned));
            assert_eq!(exposed_key(&resolved), expected_key.map(str::to_owned));
        }
        Expected::UnknownProvider => {
            assert!(matches!(
                result.unwrap_err(),
                ProviderDefaultsError::UnknownProvider { .. }
            ));
        }
        Expected::MissingMantleEndpoint => {
            assert!(matches!(
                result.unwrap_err(),
                ProviderDefaultsError::MissingMantleEndpoint
            ));
        }
        Expected::InvalidApiBase => {
            assert!(matches!(
                result.unwrap_err(),
                ProviderDefaultsError::InvalidApiBase { .. }
            ));
        }
    }
}

#[test]
fn slug_reports_the_wire_name_of_each_provider_key() {
    assert_eq!(DefaultsProvider::Known(ProviderId::Groq).slug(), "groq");
    assert_eq!(
        DefaultsProvider::Known(ProviderId::Llama).slug(),
        "meta_llama"
    );
    assert_eq!(DefaultsProvider::Anyscale.slug(), "anyscale");
}
