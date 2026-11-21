use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use serde_json::Number;

use super::{block_on, parse};
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_model_info::cache::DiscoveryCache;
use crate::deployment_model_info::discovery::CatalogSnapshot;
use crate::deployment_model_info::metadata::{ModelMetadataSource, RouterMetadata};
use crate::model_group_info::provider::ContextProviderResolver;
use crate::model_group_info::sources::{
    DeploymentInfo as _, ProviderFailure, ProviderRequest, ProviderResolver as _,
};

// The dynamic metadata source behind RouterMetadata, answering one builtin model name.
struct InfoSource;

impl ModelMetadataSource for InfoSource {
    type Error = Infallible;

    fn get_model_info(
        &self,
        model: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Infallible>> + Send {
        let answer = (model == "b846-builtin")
            .then(|| parse(r#"{"key":"b846-builtin","max_input_tokens":20}"#));
        ready(Ok(answer))
    }
}

// get_llm_provider behind the trait: a registered custom prefix names the provider, and a model it cannot resolve is the BadRequest the aggregation logs.
#[test]
fn provider_resolution() {
    let model_catalog = ModelCatalog::parse(b"{}", UnknownFields::Reject).expect("catalog");
    let known_model_names = BTreeSet::new();
    let mut custom_providers = BTreeSet::new();
    custom_providers.insert("b846-custom".to_owned());
    let environment = BTreeMap::new();
    let resolution_context = ResolutionContext {
        catalog: &model_catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let resolver = ContextProviderResolver {
        resolution: &resolution_context,
    };
    let answer = resolver
        .resolve_provider(&ProviderRequest {
            model: "b846-custom/b846-model",
            custom_llm_provider: None,
        })
        .expect("provider");
    assert_eq!(answer.provider, "b846-custom");
    assert_eq!(answer.litellm_model, "b846-model");
    let missing = resolver.resolve_provider(&ProviderRequest {
        model: "b846-nothing",
        custom_llm_provider: None,
    });
    assert!(matches!(
        missing,
        Err(ProviderFailure::BadRequest(message)) if message == "b846-nothing"
    ));
}

// get_deployment_model_info behind the trait: the source's answer comes back, and a name it does not know answers none.
#[test]
fn deployment_info_reads_the_source() {
    let deployments = Arc::new(DeploymentCatalog::default());
    let snapshot = CatalogSnapshot {
        catalog: &deployments,
    };
    let discovery = DiscoveryCache::for_deployments(1);
    let registry = ModelRegistry::default();
    let model_catalog = ModelCatalog::parse(b"{}", UnknownFields::Reject).expect("catalog");
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution_context = ResolutionContext {
        catalog: &model_catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let source = InfoSource;
    let metadata = RouterMetadata {
        snapshot: &snapshot,
        discovery: &discovery,
        registry: &registry,
        resolution: &resolution_context,
        source: &source,
    };
    let mut keys = ModelCostKeys::default();
    let found = block_on(metadata.info("a", "b846-builtin", &mut keys))
        .expect("infallible")
        .expect("metadata");
    assert_eq!(
        found.get("max_input_tokens"),
        Some(&Json::Number(Number::from(20_i64)))
    );
    let unknown = block_on(metadata.info("a", "b846-unknown", &mut keys)).expect("infallible");
    assert_eq!(unknown, None);
}
