use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use rstest::rstest;

use super::{json, parameters, resolution, resolver};
use crate::api::responses_provider_selection::prompting::PromptError;
use crate::api::responses_provider_selection::prompting::swap::{
    PromptSwapError, SwapRequest, model_info_after_swap, resolve_swapped_provider,
};
use crate::api::responses_provider_selection::resolution::ModelResolver;

fn expected_message(
    prompt_id: &str,
    original_model: &str,
    swapped_model: &str,
    original_provider: &str,
    swapped_provider: &str,
) -> String {
    format!(
        "prompt_id '{prompt_id}' swaps model '{original_model}' -> '{swapped_model}', which changes the provider from '{original_provider}' to '{swapped_provider}' after credentials for '{original_provider}' were already resolved. Refusing to send them to '{swapped_provider}'. Point the request at a model whose provider matches the prompt's metadata.model, or set ignore_prompt_manager_model on the prompt to keep the requested model."
    )
}

#[test]
fn absent_credentials_never_resolve_the_original_provider() {
    let resolver = resolver(&[("claude-3", resolution("claude-3", "anthropic"))]);
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "claude-3",
        custom_llm_provider: Some("openai"),
        prompt_id: Some("p-1"),
    };
    let provider = resolve_swapped_provider(&request, &parameters(&[]), &resolver)
        .expect("no resolved credentials mean no boundary");
    assert_eq!(provider, "anthropic");
    assert_eq!(resolver.models(), vec![String::from("claude-3")]);
}

#[test]
fn null_credentials_count_as_absent() {
    let resolver = resolver(&[("claude-3", resolution("claude-3", "anthropic"))]);
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "claude-3",
        custom_llm_provider: Some("openai"),
        prompt_id: None,
    };
    let kwargs = parameters(&[("api_key", json("null")), ("api_base", json("null"))]);
    let provider = resolve_swapped_provider(&request, &kwargs, &resolver)
        .expect("null credentials mean no boundary");
    assert_eq!(provider, "anthropic");
}

#[rstest]
#[case::empty_key("api_key")]
#[case::empty_base("api_base")]
fn an_empty_setting_still_counts_as_resolved(#[case] name: &str) {
    let resolver = resolver(&[("claude-3", resolution("claude-3", "anthropic"))]);
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "claude-3",
        custom_llm_provider: Some("openai"),
        prompt_id: Some("p-1"),
    };
    let kwargs = parameters(&[(name, json(r#""""#))]);
    let error = resolve_swapped_provider(&request, &kwargs, &resolver)
        .expect_err("an empty setting is a resolved credential");
    let PromptError::Swap(PromptSwapError::CredentialBoundary(boundary)) = error else {
        panic!("a provider change with resolved credentials is the credential boundary");
    };
    assert_eq!(
        boundary.message,
        expected_message("p-1", "gpt-4o", "claude-3", "openai", "anthropic")
    );
}

#[test]
fn the_same_provider_keeps_the_swap() {
    let resolver = resolver(&[("gpt-4o-mini", resolution("gpt-4o-mini", "openai"))]);
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "gpt-4o-mini",
        custom_llm_provider: Some("openai"),
        prompt_id: None,
    };
    let kwargs = parameters(&[("api_key", json(r#""sk-test""#))]);
    let provider = resolve_swapped_provider(&request, &kwargs, &resolver)
        .expect("one provider means the credentials stay");
    assert_eq!(provider, "openai");
    assert_eq!(resolver.models(), vec![String::from("gpt-4o-mini")]);
}

#[test]
fn the_original_provider_comes_from_the_model_when_the_caller_named_none() {
    let resolver = resolver(&[
        ("claude-3", resolution("claude-3", "anthropic")),
        ("gpt-4o", resolution("gpt-4o", "openai")),
    ]);
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "claude-3",
        custom_llm_provider: None,
        prompt_id: None,
    };
    let kwargs = parameters(&[("api_key", json(r#""sk-test""#))]);
    let error = resolve_swapped_provider(&request, &kwargs, &resolver)
        .expect_err("two providers cross the credential boundary");
    let PromptError::Swap(PromptSwapError::CredentialBoundary(boundary)) = error else {
        panic!("a provider change with resolved credentials is the credential boundary");
    };
    assert_eq!(
        boundary.message,
        expected_message("None", "gpt-4o", "claude-3", "openai", "anthropic")
    );
    assert_eq!(
        resolver.models(),
        vec![String::from("claude-3"), String::from("gpt-4o")]
    );
}

#[test]
fn an_unresolvable_original_model_permits_the_swap() {
    let resolver = resolver(&[("claude-3", resolution("claude-3", "anthropic"))]);
    let request = SwapRequest {
        original_model: "unknown/model",
        swapped_model: "claude-3",
        custom_llm_provider: None,
        prompt_id: None,
    };
    let kwargs = parameters(&[("api_base", json(r#""https://base.test/v1""#))]);
    let provider = resolve_swapped_provider(&request, &kwargs, &resolver)
        .expect("an original that names no provider cannot lose its credentials");
    assert_eq!(provider, "anthropic");
    assert_eq!(
        resolver.models(),
        vec![String::from("claude-3"), String::from("unknown/model")]
    );
}

#[test]
fn the_credential_boundary_is_a_bad_request() {
    let resolver = resolver(&[("claude-3", resolution("claude-3", "anthropic"))]);
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "claude-3",
        custom_llm_provider: Some("openai"),
        prompt_id: Some("p-1"),
    };
    let kwargs = parameters(&[("api_key", json(r#""sk-test""#))]);
    let error = resolve_swapped_provider(&request, &kwargs, &resolver)
        .expect_err("two providers cross the credential boundary");
    let PromptError::Swap(PromptSwapError::CredentialBoundary(boundary)) = error else {
        panic!("a provider change with resolved credentials is the credential boundary");
    };
    let class = boundary.class();
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(metadata.model.as_deref(), Some("claude-3"));
    assert_eq!(metadata.provider.as_deref(), Some("anthropic"));
    let ErrorClass::BadRequest(details) = class else {
        panic!("the credential boundary dispatches as a bad request");
    };
    assert_eq!(
        details.message,
        expected_message("p-1", "gpt-4o", "claude-3", "openai", "anthropic")
    );
}

#[rstest]
#[case::same_provider(Some("openai"), Some("openai"), true)]
#[case::both_named_by_the_model(None, None, true)]
#[case::other_provider(Some("openai"), Some("azure"), false)]
#[case::resolved_without_a_requested_provider(None, Some("openai"), false)]
#[case::requested_without_a_resolved_provider(Some("openai"), None, false)]
fn deployment_info_survives_only_an_unchanged_provider(
    #[case] requested: Option<&str>,
    #[case] resolved: Option<&str>,
    #[case] kept: bool,
) {
    let model_info = json(r#"{"id":"dep-1"}"#);
    let outcome = model_info_after_swap(requested, resolved, Some(model_info.clone()));
    assert_eq!(outcome, kept.then_some(model_info));
}

#[test]
fn absent_deployment_info_stays_absent() {
    assert_eq!(
        model_info_after_swap(Some("openai"), Some("openai"), None),
        None
    );
}

struct BrokenOriginal;

impl ModelResolver for BrokenOriginal {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        if request.model == "claude-3" {
            return Ok(ResolvedModel {
                model: request.model.to_owned(),
                provider: ProviderIdentity::Custom(String::from("anthropic")),
                api_base: None,
                api_key: None,
            });
        }
        Err(ResolutionError::EndpointWithoutProvider {
            endpoint: String::from("registered-endpoint"),
            api_base: String::from("https://base.test/v1"),
        })
    }
}

#[test]
fn an_unresolvable_swapped_model_is_a_resolution_error() {
    let resolver = resolver(&[]);
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "unknown/model",
        custom_llm_provider: Some("openai"),
        prompt_id: Some("p-1"),
    };
    let kwargs = parameters(&[("api_key", json(r#""sk-test""#))]);
    let error = resolve_swapped_provider(&request, &kwargs, &resolver)
        .expect_err("the swapped model names no provider");
    assert!(matches!(
        error,
        PromptError::Swap(PromptSwapError::Resolution(_))
    ));
}

#[test]
fn a_broken_original_resolution_fails_the_swap() {
    let request = SwapRequest {
        original_model: "gpt-4o",
        swapped_model: "claude-3",
        custom_llm_provider: None,
        prompt_id: Some("p-1"),
    };
    let kwargs = parameters(&[("api_key", json(r#""sk-test""#))]);
    let error = resolve_swapped_provider(&request, &kwargs, &BrokenOriginal)
        .expect_err("only a missing provider is allowed to keep the swap");
    let PromptError::Swap(PromptSwapError::Resolution(cause)) = error else {
        panic!("the original resolution error propagates");
    };
    assert!(matches!(
        cause,
        ResolutionError::EndpointWithoutProvider { .. }
    ));
}
