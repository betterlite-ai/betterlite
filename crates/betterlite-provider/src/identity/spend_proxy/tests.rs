#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use rstest::rstest;
use serde_json::Number;

use super::{DeploymentIdentity, ProxySelection, deployment_provider};

struct Fixture {
    catalog: Arc<ModelCatalog>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Fixture {
    fn new() -> Self {
        let mut bundled = BundledCatalog::default();
        Self {
            catalog: bundled.get(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

fn params(entries: &[(&str, Json)]) -> JsonObject {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn identity(params: &JsonObject) -> DeploymentIdentity<'_> {
    DeploymentIdentity::from_litellm_params(params)
}

const fn openai_deployment(use_litellm_proxy: bool) -> DeploymentIdentity<'static> {
    DeploymentIdentity {
        model: "openai/gpt-4o",
        custom_llm_provider: None,
        use_litellm_proxy,
    }
}

#[rstest]
#[case::environment(ProxySelection { environment: true, global: false }, false)]
#[case::global(ProxySelection { environment: false, global: true }, false)]
#[case::deployment(ProxySelection::default(), true)]
fn any_proxy_switch_names_the_proxy(#[case] selection: ProxySelection, #[case] flag: bool) {
    let fixture = Fixture::new();
    let resolution = fixture.context();
    assert_eq!(
        deployment_provider(&openai_deployment(flag), selection, &resolution).as_deref(),
        Some("litellm_proxy")
    );
}

#[test]
fn a_quiet_deployment_resolves_its_prefixed_provider() {
    let fixture = Fixture::new();
    let resolution = fixture.context();
    assert_eq!(
        deployment_provider(
            &openai_deployment(false),
            ProxySelection::default(),
            &resolution
        )
        .as_deref(),
        Some("openai")
    );
}

#[test]
fn the_declared_authenticating_provider_wins_over_resolution() {
    let fixture = Fixture::new();
    let resolution = fixture.context();
    let copilot = params(&[("model", Json::Text("github_copilot/gpt-4".to_owned()))]);
    assert_eq!(
        deployment_provider(&identity(&copilot), ProxySelection::default(), &resolution).as_deref(),
        Some("github_copilot")
    );
    let chatgpt = params(&[
        ("model", Json::Text("unregistered-name".to_owned())),
        ("custom_llm_provider", Json::Text("chatgpt".to_owned())),
    ]);
    assert_eq!(
        deployment_provider(&identity(&chatgpt), ProxySelection::default(), &resolution).as_deref(),
        Some("chatgpt")
    );
}

#[test]
fn resolves_the_custom_provider_defaults() {
    let fixture = Fixture::new();
    let resolution = fixture.context();
    let anthropic = params(&[
        ("model", Json::Text("unregistered-name".to_owned())),
        ("custom_llm_provider", Json::Text("anthropic".to_owned())),
    ]);
    assert_eq!(
        deployment_provider(
            &identity(&anthropic),
            ProxySelection::default(),
            &resolution
        )
        .as_deref(),
        Some("anthropic")
    );
}

#[test]
fn a_bare_known_model_resolves_from_the_bundled_catalog() {
    let fixture = Fixture::new();
    let resolution = fixture.context();
    let bare = params(&[("model", Json::Text("gpt-4o".to_owned()))]);
    assert_eq!(
        deployment_provider(&identity(&bare), ProxySelection::default(), &resolution).as_deref(),
        Some("openai")
    );
}

#[test]
fn an_unknown_model_has_no_provider() {
    let fixture = Fixture::new();
    let resolution = fixture.context();
    let unknown = params(&[(
        "model",
        Json::Text("totally-unknown-provider/xyz".to_owned()),
    )]);
    assert_eq!(
        deployment_provider(&identity(&unknown), ProxySelection::default(), &resolution),
        None
    );
}

#[test]
fn an_empty_custom_provider_falls_back_to_the_model() {
    let fixture = Fixture::new();
    let resolution = fixture.context();
    let empty_custom = params(&[
        ("model", Json::Text("gpt-4o".to_owned())),
        ("custom_llm_provider", Json::Text(String::new())),
    ]);
    assert_eq!(
        deployment_provider(
            &identity(&empty_custom),
            ProxySelection::default(),
            &resolution
        )
        .as_deref(),
        Some("openai")
    );
}

#[test]
fn reads_only_typed_deployment_fields() {
    let entries = params(&[
        ("model", Json::Text("openai/gpt-4o".to_owned())),
        ("custom_llm_provider", Json::Text("azure".to_owned())),
        ("use_litellm_proxy", Json::Bool(true)),
        ("api_base", Json::Text("https://example.test".to_owned())),
    ]);
    let deployment = DeploymentIdentity::from_litellm_params(&entries);
    assert_eq!(deployment.model, "openai/gpt-4o");
    assert_eq!(deployment.custom_llm_provider, Some("azure"));
    assert!(deployment.use_litellm_proxy);
    let odd = params(&[
        ("model", Json::Number(Number::from(7_i32))),
        ("custom_llm_provider", Json::Bool(false)),
        ("use_litellm_proxy", Json::Text("true".to_owned())),
    ]);
    let odd_deployment = DeploymentIdentity::from_litellm_params(&odd);
    assert_eq!(odd_deployment.model, "");
    assert_eq!(odd_deployment.custom_llm_provider, None);
    assert!(!odd_deployment.use_litellm_proxy);
}
