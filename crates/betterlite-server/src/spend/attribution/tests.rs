#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::spend_attribution::Settings;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_provider::identity::spend_proxy::DeploymentIdentity;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use rstest::rstest;

use super::{configured_model_group, model_group_provider};

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

fn deployment(model: &'static str, provider: Option<&'static str>) -> DeploymentIdentity<'static> {
    DeploymentIdentity {
        model,
        custom_llm_provider: provider,
        use_litellm_proxy: false,
    }
}

#[test]
fn infers_one_unique_non_null_provider_from_group_deployments() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let deployments = [
        deployment("gpt-4o", None),
        deployment("not-a-provider/model", None),
        deployment("openai/gpt-4o", None),
    ];
    assert_eq!(
        model_group_provider("group", &deployments, &Settings::default(), &context).as_deref(),
        Some("openai")
    );
}

#[test]
fn distinct_non_null_providers_and_empty_groups_have_no_inference() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let distinct = [
        deployment("gpt-4o", None),
        deployment("anthropic/claude", None),
    ];
    assert_eq!(
        model_group_provider("group", &distinct, &Settings::default(), &context),
        None
    );
    assert_eq!(
        model_group_provider("", &distinct, &Settings::default(), &context),
        None
    );
}

#[test]
fn an_empty_provider_set_does_not_infer_a_group_provider() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let unresolved = [deployment("not-a-provider/model", None)];
    assert_eq!(
        model_group_provider("group", &unresolved, &Settings::default(), &context),
        None
    );
}

#[rstest]
#[case::configured_group("friendly-group", true)]
#[case::configured_name_is_still_exempt("malformed name", true)]
#[case::unconfigured_malformed_name("malformed name", false)]
fn configured_groups_are_exempt_from_model_syntax(#[case] name: &str, #[case] configured: bool) {
    let recognized = if configured {
        BTreeSet::from([name.to_owned()])
    } else {
        BTreeSet::new()
    };
    let teams = BTreeSet::new();
    assert_eq!(
        configured_model_group(name, &recognized, &teams),
        configured
    );
}

#[test]
fn team_public_names_are_exempt_from_model_syntax() {
    let teams = BTreeSet::from(["public group".to_owned()]);
    assert!(configured_model_group(
        "public group",
        &BTreeSet::new(),
        &teams
    ));
    assert!(!configured_model_group(
        "private group",
        &BTreeSet::new(),
        &teams
    ));
}

#[rstest]
#[case::alias("public", true, Some("litellm_proxy"))]
#[case::missing("missing", true, None)]
#[case::detached("public", false, None)]
#[case::empty("", true, None)]
fn catalog_aliases_supply_group_deployments(
    #[case] name: &str,
    #[case] attached: bool,
    #[case] provider: Option<&str>,
) {
    let mut catalog = DeploymentCatalog::default();
    catalog.push(DeploymentSpec {
        model_name: "internal".to_owned(),
        litellm_params: serde_json::from_str(
            r#"{"model":"openai/gpt-4o","use_litellm_proxy":true}"#,
        )
        .unwrap(),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    });
    let aliases = AliasMap::from([(
        "public".to_owned(),
        AliasTarget::Name("internal".to_owned()),
    )]);
    let groups = RoutingGroupIndex::default();
    let lookup = CatalogLookup {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &|_| None,
    };
    let fixture = Fixture::new();
    let context = fixture.context();
    assert_eq!(
        super::catalog_group_provider(
            name,
            attached.then_some(&lookup),
            &Settings::default(),
            &context
        )
        .unwrap()
        .as_deref(),
        provider
    );
}

#[test]
fn empty_groups_are_never_configured() {
    let names = BTreeSet::from([String::new()]);
    assert!(!configured_model_group("", &names, &names));
    assert!(!configured_model_group("unlisted", &names, &names));
}
