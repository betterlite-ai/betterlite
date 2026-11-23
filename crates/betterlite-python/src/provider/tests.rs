#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use secrecy::SecretString;

use crate::provider::compatible::openai_compatible_info;
use crate::provider::convert::{
    CompatibleRequest, HandlerRequest, LookupRequest, LookupResult, encode, parse,
};
use crate::provider::error::ProviderError;
use crate::provider::handlers::{handle_anthropic, handle_cohere};
use crate::provider::lookup::{declared_authenticating, get_llm_provider};
use crate::provider::snapshot::ProviderSnapshot;

const CATALOG: &str = r#"{
    "gpt-4": {"litellm_provider": "openai", "mode": "chat"},
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat"},
    "command-r-plus-08-2024": {"litellm_provider": "cohere_chat", "mode": "chat"},
    "claude-2": {"litellm_provider": "anthropic", "mode": "completion"},
    "mistral/mistral-large-latest": {"litellm_provider": "mistral", "mode": "chat"}
}"#;

fn test_catalog() -> Arc<ModelCatalog> {
    Arc::new(ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap())
}

fn test_environment() -> BTreeMap<String, String> {
    [
        ("GROQ_API_KEY", "groq-key"),
        ("ANYSCALE_API_KEY", "anyscale-key"),
        ("JINA_AI_API_KEY", "jina-key"),
        ("MISTRAL_API_KEY", "mistral-key"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect()
}

fn snapshot(settings: &str) -> ProviderSnapshot {
    ProviderSnapshot::build_with(settings, test_environment(), test_catalog()).unwrap()
}

fn message_of(error: ProviderError) -> String {
    match error {
        ProviderError::BadRequest { message, .. } => message,
        ProviderError::Json { source } => {
            panic!("expected a BadRequestError, got a JSON error: {source}")
        }
        ProviderError::Encode { source } => {
            panic!("expected a BadRequestError, got an encoding error: {source}")
        }
    }
}

fn exposed(key: Option<&SecretString>) -> Option<&str> {
    key.map(secrecy::ExposeSecret::expose_secret)
}

fn lookup_request(model: &str) -> LookupRequest {
    LookupRequest {
        model: Some(model.to_owned()),
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    }
}

#[test]
fn provider_request_lookup_reads_its_fields() {
    let request: LookupRequest = parse(
        r#"{"model": "gpt-4", "custom_llm_provider": "openai", "api_base": "https://x/v1", "api_key": "sk-x"}"#,
    )
    .unwrap();
    assert_eq!(request.model.as_deref(), Some("gpt-4"));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.api_base.as_deref(), Some("https://x/v1"));
    assert_eq!(exposed(request.api_key.as_ref()), Some("sk-x"));
}

#[test]
fn provider_request_lookup_reads_an_empty_object() {
    let request: LookupRequest = parse("{}").unwrap();
    assert_eq!(request.model, None);
    assert_eq!(request.custom_llm_provider, None);
    assert_eq!(request.api_base, None);
    assert!(request.api_key.is_none());
}

#[test]
fn provider_request_lookup_rejects_malformed_text() {
    let error = parse::<LookupRequest>("{\"model\": ").unwrap_err();
    assert!(matches!(error, ProviderError::Json { .. }));
}

#[test]
fn provider_request_compatible_reads_its_fields() {
    let request: CompatibleRequest = parse(
        r#"{"model": "groq/llama3-8b-8192", "api_base": "https://x/v1", "api_key": "sk-x", "dynamic_api_key": "sk-d"}"#,
    )
    .unwrap();
    assert_eq!(request.model, "groq/llama3-8b-8192");
    assert_eq!(request.api_base.as_deref(), Some("https://x/v1"));
    assert_eq!(exposed(request.api_key.as_ref()), Some("sk-x"));
    assert_eq!(exposed(request.dynamic_api_key.as_ref()), Some("sk-d"));
}

#[test]
fn provider_request_compatible_requires_a_model() {
    let error = parse::<CompatibleRequest>("{}").unwrap_err();
    assert!(matches!(error, ProviderError::Json { .. }));
}

#[test]
fn provider_request_handler_reads_an_absent_model() {
    let request: HandlerRequest = parse(r#"{"custom_llm_provider": "chatgpt"}"#).unwrap();
    assert_eq!(request.model, None);
    assert_eq!(request.custom_llm_provider.as_deref(), Some("chatgpt"));
}

#[test]
fn provider_request_settings_read_the_snapshot() {
    let settings = snapshot(
        r#"{
            "custom_provider_map": [{"provider": "myprovider"}, {"provider": "myprovider"}, {"provider": "other"}],
            "model_list_set": ["gpt-4", "gpt-4"],
            "api_key": "sk-global",
            "api_base": "https://global/v1",
            "xai_key": "sk-xai",
            "inception_key": "sk-inception",
            "lemonade_key": "sk-lemonade"
        }"#,
    );
    assert_eq!(
        settings.custom_providers,
        BTreeSet::from(["myprovider".to_owned(), "other".to_owned()])
    );
    assert_eq!(
        settings.known_model_names,
        BTreeSet::from(["gpt-4".to_owned()])
    );
    assert_eq!(
        exposed(settings.globals.api_key.as_ref()),
        Some("sk-global")
    );
    assert_eq!(
        settings.globals.api_base.as_deref(),
        Some("https://global/v1")
    );
    assert_eq!(exposed(settings.globals.xai_key.as_ref()), Some("sk-xai"));
    assert_eq!(
        exposed(settings.globals.inception_key.as_ref()),
        Some("sk-inception")
    );
    assert_eq!(
        exposed(settings.globals.lemonade_key.as_ref()),
        Some("sk-lemonade")
    );
}

#[test]
fn provider_request_settings_accept_an_empty_object() {
    let settings = snapshot("{}");
    assert!(settings.custom_providers.is_empty());
    assert!(settings.known_model_names.is_empty());
    assert!(settings.globals.api_key.is_none());
}

#[test]
fn provider_request_settings_reject_a_bad_entry() {
    let error = ProviderSnapshot::build_with(
        "{\"custom_provider_map\": [{\"handler\": 1}]}",
        test_environment(),
        test_catalog(),
    )
    .unwrap_err();
    assert!(matches!(error, ProviderError::Json { .. }));
}

#[test]
fn provider_result_writes_the_four_values() {
    let result = LookupResult {
        model: "llama3-8b-8192".to_owned(),
        provider: "groq".to_owned(),
        api_key: Some(SecretString::from("sk-x")),
        api_base: Some("https://api.groq.com/openai/v1".to_owned()),
    };
    assert_eq!(
        result.to_json().unwrap(),
        r#"["llama3-8b-8192","groq","sk-x","https://api.groq.com/openai/v1"]"#
    );
}

#[test]
fn provider_result_writes_the_handler_pair() {
    let moved = (
        "command-r-plus-08-2024".to_owned(),
        Some("cohere_chat".to_owned()),
    );
    assert_eq!(
        encode(&moved).unwrap(),
        r#"["command-r-plus-08-2024","cohere_chat"]"#
    );
    let kept = ("gpt-4".to_owned(), None::<String>);
    assert_eq!(encode(&kept).unwrap(), r#"["gpt-4",null]"#);
}

#[test]
fn provider_result_writes_the_declared_provider() {
    assert_eq!(encode(&Some("chatgpt")).unwrap(), r#""chatgpt""#);
    assert_eq!(encode(&None::<&str>).unwrap(), "null");
}

#[test]
fn provider_lookup_resolves_a_model_the_catalog_knows() {
    let request = lookup_request("gpt-4");
    let resolved = get_llm_provider(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "gpt-4");
    assert_eq!(resolved.provider, "openai");
    assert!(resolved.api_key.is_none());
    assert_eq!(resolved.api_base, None);
}

#[test]
fn provider_lookup_completes_a_declared_provider() {
    let mut request = lookup_request("llama3-8b-8192");
    request.custom_llm_provider = Some("groq".to_owned());
    let resolved = get_llm_provider(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "llama3-8b-8192");
    assert_eq!(resolved.provider, "groq");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("groq-key"));
    assert_eq!(
        resolved.api_base.as_deref(),
        Some("https://api.groq.com/openai/v1")
    );
}

#[test]
fn provider_lookup_returns_a_registered_custom_provider() {
    let settings = snapshot(r#"{"custom_provider_map": [{"provider": "myprovider"}]}"#);
    let request = lookup_request("myprovider/model-x");
    let resolved = get_llm_provider(&settings, &request).unwrap();
    assert_eq!(resolved.model, "model-x");
    assert_eq!(resolved.provider, "myprovider");
    assert_eq!(resolved.api_base, None);
}

#[test]
fn provider_lookup_keeps_the_caller_credentials_for_an_unlisted_provider() {
    let mut request = lookup_request("openai/gpt-4");
    request.api_base = Some("https://x/v1".to_owned());
    request.api_key = Some(SecretString::from("sk-x"));
    let resolved = get_llm_provider(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "gpt-4");
    assert_eq!(resolved.provider, "openai");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("sk-x"));
    assert_eq!(resolved.api_base.as_deref(), Some("https://x/v1"));
}

#[test]
fn provider_lookup_rejects_a_missing_model() {
    let request = LookupRequest::default();
    let error = get_llm_provider(&snapshot("{}"), &request).unwrap_err();
    assert_eq!(
        message_of(error),
        "GetLLMProvider Exception - model parameter is required but was None. Please provide a valid model name.\n\noriginal model: None"
    );
}

#[test]
fn provider_lookup_reports_an_unknown_model() {
    let request = lookup_request("unknown-model-xyz");
    let error = get_llm_provider(&snapshot("{}"), &request).unwrap_err();
    assert_eq!(
        message_of(error),
        "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=unknown-model-xyz\n Pass model as E.g. For 'Huggingface' inference endpoints pass in `completion(model='huggingface/starcoder',..)` Learn more: https://docs.litellm.ai/docs/providers"
    );
}

#[test]
fn provider_compatible_resolves_a_known_provider() {
    let request: CompatibleRequest = parse(r#"{"model": "groq/llama3-8b-8192"}"#).unwrap();
    let resolved = openai_compatible_info(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "llama3-8b-8192");
    assert_eq!(resolved.provider, "groq");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("groq-key"));
    assert_eq!(
        resolved.api_base.as_deref(),
        Some("https://api.groq.com/openai/v1")
    );
}

#[test]
fn provider_compatible_falls_through_to_the_second_table() {
    let request: CompatibleRequest = parse(r#"{"model": "jina_ai/embed"}"#).unwrap();
    let resolved = openai_compatible_info(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "embed");
    assert_eq!(resolved.provider, "jina_ai");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("jina-key"));
    assert_eq!(resolved.api_base.as_deref(), Some("https://api.jina.ai/v1"));
}

#[test]
fn provider_compatible_resolves_anyscale() {
    let request: CompatibleRequest = parse(r#"{"model": "anyscale/foo"}"#).unwrap();
    let resolved = openai_compatible_info(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "foo");
    assert_eq!(resolved.provider, "anyscale");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("anyscale-key"));
    assert_eq!(
        resolved.api_base.as_deref(),
        Some("https://api.endpoints.anyscale.com/v1")
    );
}

#[test]
fn provider_compatible_keeps_the_dynamic_key_for_an_unlisted_provider() {
    let request: CompatibleRequest = parse(
        r#"{"model": "mystery/model", "api_base": "https://y/v1", "api_key": "sk-a", "dynamic_api_key": "sk-d"}"#,
    )
    .unwrap();
    let resolved = openai_compatible_info(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "model");
    assert_eq!(resolved.provider, "mystery");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("sk-d"));
    assert_eq!(resolved.api_base.as_deref(), Some("https://y/v1"));
}

#[test]
fn provider_compatible_falls_back_to_the_api_key() {
    let request: CompatibleRequest =
        parse(r#"{"model": "mystery/model", "api_key": "sk-a"}"#).unwrap();
    let resolved = openai_compatible_info(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.provider, "mystery");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("sk-a"));
    assert_eq!(resolved.api_base, None);
}

#[test]
fn provider_compatible_keeps_the_dynamic_key_for_a_known_provider_without_a_table() {
    let request: CompatibleRequest =
        parse(r#"{"model": "openai/gpt-4", "api_key": "sk-a", "dynamic_api_key": "sk-d"}"#)
            .unwrap();
    let resolved = openai_compatible_info(&snapshot("{}"), &request).unwrap();
    assert_eq!(resolved.model, "gpt-4");
    assert_eq!(resolved.provider, "openai");
    assert_eq!(exposed(resolved.api_key.as_ref()), Some("sk-d"));
}

#[test]
fn provider_compatible_rejects_a_model_without_a_prefix() {
    let request: CompatibleRequest = parse(r#"{"model": "gpt-4"}"#).unwrap();
    let error = openai_compatible_info(&snapshot("{}"), &request).unwrap_err();
    assert_eq!(
        message_of(error),
        "GetLLMProvider Exception - gpt-4 has no provider prefix\n\noriginal model: gpt-4"
    );
}

#[test]
fn provider_handler_moves_a_cohere_chat_model() {
    let request: HandlerRequest = parse(r#"{"model": "cohere/command-r-plus-08-2024"}"#).unwrap();
    assert_eq!(
        handle_cohere(&snapshot("{}"), &request),
        (
            "command-r-plus-08-2024".to_owned(),
            Some("cohere_chat".to_owned())
        )
    );
}

#[test]
fn provider_handler_moves_an_anthropic_text_model() {
    let request: HandlerRequest = parse(r#"{"model": "anthropic/claude-2"}"#).unwrap();
    assert_eq!(
        handle_anthropic(&request),
        ("claude-2".to_owned(), Some("anthropic_text".to_owned()))
    );
}

#[test]
fn provider_handler_keeps_an_anthropic_chat_model() {
    let request: HandlerRequest = parse(r#"{"model": "anthropic/claude-3-5-sonnet"}"#).unwrap();
    assert_eq!(
        handle_anthropic(&request),
        ("anthropic/claude-3-5-sonnet".to_owned(), None)
    );
}

#[test]
fn provider_declared_reads_the_model_prefix() {
    let request: HandlerRequest = parse(r#"{"model": "chatgpt/x"}"#).unwrap();
    assert_eq!(
        declared_authenticating(&request),
        Some("chatgpt".to_owned())
    );
}

#[test]
fn provider_declared_prefers_the_declaration() {
    let request: HandlerRequest =
        parse(r#"{"model": "x", "custom_llm_provider": "github_copilot"}"#).unwrap();
    assert_eq!(
        declared_authenticating(&request),
        Some("github_copilot".to_owned())
    );
}

#[test]
fn provider_declared_ignores_another_provider() {
    let request: HandlerRequest = parse(r#"{"model": "gpt-4"}"#).unwrap();
    assert_eq!(declared_authenticating(&request), None);
    let absent: HandlerRequest = parse("{}").unwrap();
    assert_eq!(declared_authenticating(&absent), None);
}
