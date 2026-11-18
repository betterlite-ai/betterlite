use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;

use crate::prometheus::failure::provider::{ProviderRequestData, api_provider_from_request_data};
use crate::prometheus::failure::tests::support::{capture_logs, metadata};

const CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai"},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

#[test]
fn the_litellm_params_provider_wins() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let params = metadata(&[("custom_llm_provider", "openai")]);
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("claude-sonnet-4-5"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn the_standard_logging_provider_comes_next() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let params = metadata(&[]);
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("gpt-4o"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn the_model_is_inferred_when_no_provider_is_declared() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let params = metadata(&[]);
    let standard_logging = metadata(&[]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("gpt-4o"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn the_litellm_params_model_is_inferred_before_the_request_model() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let params = metadata(&[("model", "gpt-4o")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: None,
        model: Some("claude-sonnet-4-5"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn an_empty_litellm_params_model_falls_to_the_request_model() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let params = metadata(&[("model", "")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: None,
        model: Some("claude-sonnet-4-5"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn an_empty_declared_provider_falls_through() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let params = metadata(&[("custom_llm_provider", "")]);
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: Some(&params),
        standard_logging: Some(&standard_logging),
        model: Some("gpt-4o"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn absent_maps_still_reach_the_declared_provider() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let standard_logging = metadata(&[("custom_llm_provider", "anthropic")]);
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: Some(&standard_logging),
        model: None,
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("anthropic"));
}

#[test]
fn a_prefixed_model_names_its_declared_provider() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some("openai/gpt-4o"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider.as_deref(), Some("openai"));
}

#[test]
fn an_unknown_model_names_no_provider() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some("brand-new-unknown"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider, None);
}

#[test]
fn an_unknown_model_leaves_a_diagnostic_debug_record() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let (logs, guard) = capture_logs();
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some("brand-new-unknown"),
    };
    let provider = api_provider_from_request_data(&request, &context);
    drop(guard);
    assert_eq!(provider, None);
    assert!(logs.text().contains("could not infer api_provider"));
}

#[test]
fn no_model_and_no_declared_provider_names_none() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: None,
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider, None);
}

#[test]
fn an_empty_model_names_no_provider() {
    let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
        .expect("the test catalog parses");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let context = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let request = ProviderRequestData {
        litellm_params: None,
        standard_logging: None,
        model: Some(""),
    };
    let provider = api_provider_from_request_data(&request, &context);
    assert_eq!(provider, None);
}
