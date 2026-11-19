use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelRegistry};
use rstest::rstest;
use serde_json::Number;

use super::fixtures::{
    Events, FakeSource, Lookups, Resolver, Unmapped, catalog, limits, object, record, registry,
    snapshot, spec,
};
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_model_info::cache::DiscoveryCache;
use crate::deployment_model_info::deployment_info::{
    get_deployment_model_info, get_model_group, get_model_info,
};
use crate::deployment_model_info::error::RouterMetadataError;
use crate::deployment_model_info::metadata::RouterMetadata;
use crate::deployment_model_info::router_info::{SuppliedDeployment, get_router_model_info};
use crate::model_catalog_lookup::error::LookupError;

type Outcome = Result<Option<JsonObject>, RouterMetadataError<Unmapped>>;

struct Case {
    catalog: Arc<DeploymentCatalog>,
    discovery: DiscoveryCache,
    registry: ModelRegistry,
    resolver: Resolver,
    source: FakeSource,
}

impl Case {
    fn new(catalog: Arc<DeploymentCatalog>, source: FakeSource, registry: ModelRegistry) -> Self {
        let discovery = DiscoveryCache::for_deployments(catalog.models.len());
        Self {
            catalog,
            discovery,
            registry,
            resolver: Resolver::new(&[]),
            source,
        }
    }

    fn plain(catalog: Arc<DeploymentCatalog>, source: FakeSource) -> Self {
        Self::new(catalog, source, ModelRegistry::default())
    }

    async fn merged(
        &self,
        deployment: Option<SuppliedDeployment<'_>>,
        received: &str,
        id: Option<&str>,
    ) -> Outcome {
        let snapshot = snapshot(&self.catalog);
        let resolution = self.resolver.context();
        let metadata = RouterMetadata {
            snapshot: &snapshot,
            discovery: &self.discovery,
            registry: &self.registry,
            resolution: &resolution,
            source: &self.source,
        };
        let mut keys = ModelCostKeys::default();
        get_router_model_info(&metadata, deployment, received, id, &mut keys).await
    }

    async fn info(&self, model_id: &str, model_name: &str) -> Option<JsonObject> {
        let snapshot = snapshot(&self.catalog);
        let resolution = self.resolver.context();
        let metadata = RouterMetadata {
            snapshot: &snapshot,
            discovery: &self.discovery,
            registry: &self.registry,
            resolution: &resolution,
            source: &self.source,
        };
        let mut keys = ModelCostKeys::default();
        get_deployment_model_info(&metadata, model_id, model_name, &mut keys).await
    }
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

#[tokio::test]
async fn resolved_id_replaces_supplied() {
    let catalog = catalog(vec![
        spec("g", r#"{"model":"openai/first"}"#, r#"{"id":"one"}"#),
        spec(
            "g",
            r#"{"model":"openai/second"}"#,
            r#"{"id":"two","mode":"chat"}"#,
        ),
    ]);
    let case = Case::plain(
        Arc::clone(&catalog),
        FakeSource::answering(&[("openai/second", r#"{"max_input_tokens":5}"#)]),
    );
    let supplied = SuppliedDeployment::from_spec(&catalog.models[0]);
    let info = case
        .merged(Some(supplied), "g", Some("two"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(case.source.seen(), vec!["openai/second".to_owned()]);
    assert_eq!(info.get("mode"), Some(&Json::Text("chat".to_owned())));
}

#[tokio::test]
async fn unknown_id_retains_supplied() {
    let catalog = catalog(vec![spec(
        "g",
        r#"{"model":"openai/first"}"#,
        r#"{"id":"one"}"#,
    )]);
    let case = Case::plain(
        Arc::clone(&catalog),
        FakeSource::answering(&[("openai/first", r#"{"max_input_tokens":5}"#)]),
    );
    let supplied = SuppliedDeployment::from_spec(&catalog.models[0]);
    let info = case
        .merged(Some(supplied), "g", Some("missing"))
        .await
        .unwrap();
    assert!(info.is_some());
    assert_eq!(case.source.seen(), vec!["openai/first".to_owned()]);
}

#[tokio::test]
async fn missing_deployment_error() {
    let catalog = catalog(vec![]);
    let case = Case::plain(catalog, FakeSource::answering(&[]));
    let outcome = case.merged(None, "g", None).await;
    match outcome {
        Err(RouterMetadataError::DeploymentNotFound) => {}
        _ => panic!("expected the missing deployment error"),
    }
    assert_eq!(
        RouterMetadataError::<Unmapped>::DeploymentNotFound.to_string(),
        "Deployment not found"
    );
}

#[tokio::test]
async fn malformed_params_error_and_id() {
    let catalog = catalog(vec![]);
    let case = Case::plain(catalog, FakeSource::answering(&[]));
    let info = object(r#"{"id":"one"}"#);
    let absent = SuppliedDeployment {
        model_info: &info,
        litellm_params: None,
    };
    let error = case.merged(Some(absent), "g", None).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Deployment missing valid litellm_params. Got: NoneType, deployment_id: one"
    );
    let model_less = object(r#"{"api_base":"x"}"#);
    let without_model = SuppliedDeployment {
        model_info: &info,
        litellm_params: Some(&model_less),
    };
    let error = case
        .merged(Some(without_model), "g", None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Deployment missing valid litellm_params. Got: dict, deployment_id: one"
    );
    let anonymous = object(r#"{}"#);
    let no_id = SuppliedDeployment {
        model_info: &anonymous,
        litellm_params: None,
    };
    let error = case.merged(Some(no_id), "g", None).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Deployment missing valid litellm_params. Got: NoneType, deployment_id: unknown"
    );
}

#[tokio::test]
async fn base_model_none_precedence() {
    let catalog = catalog(vec![]);
    let case = Case::plain(
        catalog,
        FakeSource::answering(&[
            ("azure/gpt-4", r#"{"max_input_tokens":1}"#),
            ("azure/other", r#"{"max_input_tokens":2}"#),
        ]),
    );
    let info = object(r#"{"id":"one","base_model":"azure/gpt-4"}"#);
    let params = object(
        r#"{"model":"azure/deploy","custom_llm_provider":"azure","base_model":"azure/other","api_base":"https://example.openai.azure.com","api_version":"2024-02-01"}"#,
    );
    let supplied = SuppliedDeployment {
        model_info: &info,
        litellm_params: Some(&params),
    };
    assert!(
        case.merged(Some(supplied), "g", None)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(case.source.seen(), vec!["azure/gpt-4".to_owned()]);
    let nulled = object(r#"{"id":"one","base_model":null}"#);
    let from_params = SuppliedDeployment {
        model_info: &nulled,
        litellm_params: Some(&params),
    };
    assert!(
        case.merged(Some(from_params), "g", None)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        case.source.seen(),
        vec!["azure/gpt-4".to_owned(), "azure/other".to_owned()]
    );
}

#[tokio::test]
async fn nonazure_resolved_model_overrides_base() {
    let catalog = catalog(vec![]);
    let case = Case::plain(
        catalog,
        FakeSource::answering(&[("openai/gpt-4o", r#"{"max_input_tokens":5}"#)]),
    );
    let info = object(r#"{"id":"one","base_model":"openai/gpt-3.5-turbo"}"#);
    let params = object(r#"{"model":"openai/gpt-4o"}"#);
    let supplied = SuppliedDeployment {
        model_info: &info,
        litellm_params: Some(&params),
    };
    assert!(
        case.merged(Some(supplied), "g", None)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(case.source.seen(), vec!["openai/gpt-4o".to_owned()]);
}

fn azure_case(entry: &str) -> Case {
    let catalog = catalog(vec![]);
    Case::new(
        catalog,
        FakeSource::answering(&[("azure/deploy", r#"{"max_input_tokens":7}"#)]),
        registry(entry),
    )
}

fn azure_supplied<'row>(
    info: &'row JsonObject,
    params: &'row JsonObject,
) -> SuppliedDeployment<'row> {
    SuppliedDeployment {
        model_info: info,
        litellm_params: Some(params),
    }
}

#[rstest]
#[case::input_tokens_only(r#"{"azure/deploy":{"max_input_tokens":4096}}"#)]
#[case::zero_cost_with_a_token_limit(
    r#"{"azure/deploy":{"max_tokens":1,"input_cost_per_token":0}}"#
)]
#[tokio::test]
async fn azure_positive_input_or_context_fallback_debug(#[case] entry: &str) {
    let info = object(r#"{"id":"one"}"#);
    let params = object(
        r#"{"model":"azure/deploy","custom_llm_provider":"azure","api_base":"https://example.openai.azure.com","api_version":"2024-02-01"}"#,
    );
    let case = azure_case(entry);
    let events = Events::default();
    let _guard = tracing::subscriber::set_default(events.clone());
    let outcome = case
        .merged(Some(azure_supplied(&info, &params)), "g", None)
        .await;
    assert!(outcome.unwrap().is_some());
    let debug = events.at("DEBUG");
    assert!(
        debug
            .iter()
            .any(|message| message.contains("has no base_model set; using 'azure/deploy'"))
    );
    assert!(events.at("ERROR").is_empty());
}

#[rstest]
#[case::zero_stub(
    r#"{"azure/deploy":{"max_input_tokens":0,"max_tokens":0,"input_cost_per_token":0}}"#
)]
#[case::output_only_with_a_bool(
    r#"{"azure/deploy":{"output_cost_per_token":0.5,"max_tokens":false}}"#
)]
#[tokio::test]
async fn azure_zero_stub_or_output_only_error_continues(#[case] entry: &str) {
    let info = object(r#"{"id":"one"}"#);
    let params = object(
        r#"{"model":"azure/deploy","custom_llm_provider":"azure","api_base":"https://example.openai.azure.com","api_version":"2024-02-01"}"#,
    );
    let case = azure_case(entry);
    let events = Events::default();
    let _guard = tracing::subscriber::set_default(events.clone());
    let outcome = case
        .merged(Some(azure_supplied(&info, &params)), "g", None)
        .await;
    assert!(outcome.unwrap().is_some());
    let errors = events.at("ERROR");
    assert!(
        errors
            .iter()
            .any(|message| message.contains("Could not identify azure model 'deploy'"))
    );
    assert!(events.at("DEBUG").is_empty());
}

#[tokio::test]
async fn wildcard_same_id_and_bad_candidate() {
    let wildcard = catalog(vec![spec(
        "openai/*",
        r#"{"model":"openai/gpt-4o"}"#,
        r#"{"id":"wild"}"#,
    )]);
    let case = Case::plain(
        Arc::clone(&wildcard),
        FakeSource::answering(&[
            ("openai/gpt-4o", r#"{"max_input_tokens":5}"#),
            ("openai/*", r#"{"max_input_tokens":6}"#),
        ]),
    );
    let info = object(r#"{"id":"wild"}"#);
    let params = object(r#"{"model":"openai/*","custom_llm_provider":"openai"}"#);
    let supplied = azure_supplied(&info, &params);
    assert!(
        case.merged(Some(supplied), "openai/gpt-4-turbo", None)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(case.source.seen(), vec!["openai/gpt-4o".to_owned()]);
    let broken = catalog(vec![spec("openai/*", r#"{}"#, r#"{"id":"wild"}"#)]);
    let bad = Case::plain(
        Arc::clone(&broken),
        FakeSource::answering(&[("openai/*", r#"{"max_input_tokens":6}"#)]),
    );
    let info = object(r#"{"id":"wild"}"#);
    let params = object(r#"{"model":"openai/*","custom_llm_provider":"openai"}"#);
    let supplied = azure_supplied(&info, &params);
    assert!(
        bad.merged(Some(supplied), "openai/gpt-4-turbo", None)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(bad.source.seen(), vec!["openai/*".to_owned()]);
}

#[tokio::test]
async fn provider_prefix_once() {
    let catalog = catalog(vec![]);
    let case = Case::plain(
        catalog,
        FakeSource::answering(&[("openai/gpt-4o", r#"{"max_input_tokens":5}"#)]),
    );
    let info = object(r#"{"id":"one"}"#);
    let params = object(r#"{"model":"openai/gpt-4o"}"#);
    let supplied = azure_supplied(&info, &params);
    assert!(
        case.merged(Some(supplied), "g", None)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(case.source.seen(), vec!["openai/gpt-4o".to_owned()]);
}

#[tokio::test]
async fn model_lookup_none_or_error() {
    let catalog = catalog(vec![]);
    let empty = Case::plain(Arc::clone(&catalog), FakeSource::answering(&[]));
    let info = object(r#"{"id":"one"}"#);
    let params = object(r#"{"model":"openai/gpt-4o"}"#);
    let supplied = azure_supplied(&info, &params);
    assert!(
        empty
            .merged(Some(supplied), "g", None)
            .await
            .unwrap()
            .is_none()
    );
    let failing = Case::plain(catalog, FakeSource::failing());
    let supplied = azure_supplied(&info, &params);
    let error = failing.merged(Some(supplied), "g", None).await.unwrap_err();
    assert_eq!(error.to_string(), "the model is unmapped");
    assert!(matches!(
        error,
        RouterMetadataError::Source { source: Unmapped }
    ));
}

#[tokio::test]
async fn configured_false_zero_empty_win() {
    let catalog = catalog(vec![]);
    let case = Case::plain(
        catalog,
        FakeSource::answering(&[(
            "openai/gpt-4o",
            r#"{"max_input_tokens":100,"max_output_tokens":5,"supports_function_calling":true,"mode":"chat"}"#,
        )]),
    );
    let info = object(
        r#"{"id":"one","max_input_tokens":0,"max_output_tokens":null,"supports_function_calling":false,"mode":""}"#,
    );
    let params = object(r#"{"model":"openai/gpt-4o"}"#);
    let supplied = azure_supplied(&info, &params);
    let merged = case
        .merged(Some(supplied), "g", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(merged.get("max_input_tokens"), Some(&number(0)));
    assert_eq!(merged.get("max_output_tokens"), Some(&number(5)));
    assert_eq!(
        merged.get("supports_function_calling"),
        Some(&Json::Bool(false))
    );
    assert_eq!(merged.get("mode"), Some(&Json::Text(String::new())));
}

#[tokio::test]
async fn source_metadata_unmodified() {
    let catalog = catalog(vec![]);
    let case = Case::plain(
        Arc::clone(&catalog),
        FakeSource::answering(&[("openai/gpt-4o", r#"{"max_input_tokens":5,"mode":"chat"}"#)]),
    );
    let info = object(r#"{"id":"one","mode":"override"}"#);
    let params = object(r#"{"model":"openai/gpt-4o"}"#);
    let supplied = azure_supplied(&info, &params);
    let merged = case
        .merged(Some(supplied), "g", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(merged.get("mode"), Some(&Json::Text("override".to_owned())));
    let stored = case.source.answers.get("openai/gpt-4o").unwrap();
    assert_eq!(stored.get("mode"), Some(&Json::Text("chat".to_owned())));
    let plain = object(r#"{"id":"one"}"#);
    let supplied = azure_supplied(&plain, &params);
    let merged = case
        .merged(Some(supplied), "g", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(merged.get("mode"), Some(&Json::Text("chat".to_owned())));
}

#[test]
fn raw_lookup_identity() {
    let built = catalog(vec![spec(
        "g",
        r#"{"model":"openai/first"}"#,
        r#"{"id":"one"}"#,
    )]);
    let found = get_model_info(&snapshot(&built), "one").unwrap().unwrap();
    assert!(std::ptr::eq(found, &built.models[0]));
    assert!(get_model_info(&snapshot(&built), "two").unwrap().is_none());
    let mut stale = DeploymentCatalog::default();
    stale.push(spec("g", r#"{"model":"openai/first"}"#, r#"{"id":"one"}"#));
    drop(stale.id_index.insert("ghost".to_owned(), 9));
    let stale = Arc::new(stale);
    match get_model_info(&snapshot(&stale), "ghost") {
        Err(LookupError::InvalidRow { index }) => assert_eq!(index, 9),
        _ => panic!("expected the stale index failure"),
    }
}

#[test]
fn group_by_indexed_id() {
    let built = catalog(vec![
        spec("g", r#"{"model":"openai/first"}"#, r#"{"id":"one"}"#),
        spec("g", r#"{"model":"openai/second"}"#, r#"{"id":"two"}"#),
    ]);
    let lookups = Lookups::default();
    let lookup = lookups.lookup(&built);
    let group = get_model_group(&lookup, "one").unwrap().unwrap();
    assert_eq!(group.len(), 2);
    assert_eq!(
        group[1].litellm_params.get("model"),
        Some(&Json::Text("openai/second".to_owned()))
    );
    assert!(get_model_group(&lookup, "absent").unwrap().is_none());
}

#[tokio::test]
async fn custom_builtin_base_combinations() {
    let catalog = catalog(vec![]);
    let case = Case::new(
        Arc::clone(&catalog),
        FakeSource::answering(&[
            (
                "custom-model",
                r#"{"max_input_tokens":20,"key":"custom-model"}"#,
            ),
            (
                "openai/base",
                r#"{"key":"openai/base","max_output_tokens":99}"#,
            ),
        ]),
        registry(
            r#"{"custom-id":{"max_input_tokens":10},"based-id":{"base_model":"openai/base","max_input_tokens":30}}"#,
        ),
    );
    let custom = case.info("custom-id", "unknown-name").await.unwrap();
    assert_eq!(custom.get("max_input_tokens"), Some(&number(10)));
    let builtin = case.info("absent-id", "custom-model").await.unwrap();
    assert_eq!(builtin.get("max_input_tokens"), Some(&number(20)));
    let both = case.info("custom-id", "custom-model").await.unwrap();
    assert_eq!(both.get("max_input_tokens"), Some(&number(10)));
    let based = case.info("based-id", "unknown-name").await.unwrap();
    assert_eq!(based.get("max_input_tokens"), Some(&number(30)));
    assert_eq!(based.get("max_output_tokens"), Some(&number(99)));
    assert_eq!(
        based.get("base_model"),
        Some(&Json::Text("openai/base".to_owned()))
    );
    assert!(case.info("absent-id", "unknown-name").await.is_none());
}

#[tokio::test]
async fn each_lookup_failure_swallowed() {
    let catalog = catalog(vec![]);
    let case = Case::new(
        Arc::clone(&catalog),
        FakeSource::failing(),
        registry(r#"{"custom-id":{"max_input_tokens":10}}"#),
    );
    assert_eq!(
        case.info("custom-id", "any")
            .await
            .unwrap()
            .get("max_input_tokens"),
        Some(&number(10))
    );
    assert!(case.info("other-id", "any").await.is_none());
}

#[tokio::test]
async fn no_custom_id_no_discovery() {
    let catalog = catalog(vec![spec(
        "g",
        r#"{"model":"openai/first"}"#,
        r#"{"id":"absent-id"}"#,
    )]);
    let mut case = Case::plain(
        Arc::clone(&catalog),
        FakeSource::answering(&[(
            "custom-model",
            r#"{"key":"custom-model","max_input_tokens":20}"#,
        )]),
    );
    case.discovery.store(
        "absent-id",
        record(&catalog, 0, limits(None, None, Some(4096))),
    );
    let info = case.info("absent-id", "custom-model").await.unwrap();
    assert_eq!(info.get("max_input_tokens"), Some(&number(20)));
    assert!(info.get("max_output_tokens").is_none());
    let mut with_custom = Case::new(
        Arc::clone(&catalog),
        FakeSource::answering(&[(
            "custom-model",
            r#"{"key":"custom-model","max_input_tokens":20}"#,
        )]),
        registry(r#"{"absent-id":{"max_input_tokens":1}}"#),
    );
    with_custom.discovery.store(
        "absent-id",
        record(&catalog, 0, limits(None, None, Some(4096))),
    );
    let info = with_custom.info("absent-id", "custom-model").await.unwrap();
    assert_eq!(info.get("max_output_tokens"), Some(&number(4096)));
}

#[tokio::test]
async fn numeric_strings_and_shallow_nested_merge() {
    let catalog = catalog(vec![]);
    let case = Case::new(
        Arc::clone(&catalog),
        FakeSource::answering(&[(
            "custom-model",
            r#"{"key":"custom-model","max_input_tokens":1,"provider_specific_entry":{"b":2,"shared":"builtin"}}"#,
        )]),
        registry(
            r#"{"custom-id":{"max_input_tokens":"8192","provider_specific_entry":{"a":1,"shared":"custom"}}}"#,
        ),
    );
    let info = case.info("custom-id", "custom-model").await.unwrap();
    assert_eq!(info.get("max_input_tokens"), Some(&number(8192)));
    let nested = match info.get("provider_specific_entry") {
        Some(Json::Object(fields)) => fields.clone(),
        _ => panic!("expected the nested map"),
    };
    assert_eq!(nested.get("a"), Some(&number(1)));
    assert_eq!(nested.get("b"), Some(&number(2)));
    assert_eq!(nested.get("shared"), Some(&Json::Text("custom".to_owned())));
}

#[tokio::test]
async fn absent_prices_become_null() {
    let catalog = catalog(vec![]);
    let case = Case::new(
        Arc::clone(&catalog),
        FakeSource::failing(),
        registry(r#"{"custom-id":{"max_input_tokens":10}}"#),
    );
    let info = case.info("custom-id", "unknown-name").await.unwrap();
    assert_eq!(info.get("input_cost_per_token"), Some(&Json::Null));
    assert_eq!(info.get("output_cost_per_token"), Some(&Json::Null));
}

#[tokio::test]
async fn nonobject_cost_entry_truthiness() {
    let catalog = catalog(vec![]);
    let truthy = Case::new(
        Arc::clone(&catalog),
        FakeSource::failing(),
        registry(r#"{"custom-id":5}"#),
    );
    assert!(truthy.info("custom-id", "unknown-name").await.is_none());
    let falsy = Case::new(
        Arc::clone(&catalog),
        FakeSource::failing(),
        registry(r#"{"custom-id":0}"#),
    );
    let info = falsy.info("custom-id", "unknown-name").await.unwrap();
    assert!(info.is_empty());
}

#[tokio::test]
async fn base_key_participates_in_price_omission() {
    let catalog = catalog(vec![]);
    let case = Case::new(
        Arc::clone(&catalog),
        FakeSource::answering(&[(
            "openai/base",
            r#"{"key":"openai/base","max_output_tokens":99}"#,
        )]),
        registry(
            r#"{"custom-id":{"base_model":"openai/base"},"openai/base":{"input_cost_per_token":0.5}}"#,
        ),
    );
    let info = case.info("custom-id", "unknown-name").await.unwrap();
    assert_eq!(info.get("max_output_tokens"), Some(&number(99)));
    assert!(info.get("input_cost_per_token").is_none());
    assert!(info.get("output_cost_per_token").is_none());
}

#[tokio::test]
async fn explicit_zero_or_null_price_key_present() {
    let catalog = catalog(vec![]);
    let zero = Case::new(
        Arc::clone(&catalog),
        FakeSource::failing(),
        registry(r#"{"custom-id":{"input_cost_per_token":0}}"#),
    );
    let info = zero.info("custom-id", "unknown-name").await.unwrap();
    assert_eq!(info.get("input_cost_per_token"), Some(&number(0)));
    assert!(info.get("output_cost_per_token").is_none());
    let nulled = Case::new(
        catalog,
        FakeSource::failing(),
        registry(r#"{"custom-id":{"output_cost_per_token":null}}"#),
    );
    let info = nulled.info("custom-id", "unknown-name").await.unwrap();
    assert_eq!(info.get("output_cost_per_token"), Some(&Json::Null));
    assert!(info.get("input_cost_per_token").is_none());
}

#[tokio::test]
async fn discovery_overlay_and_configured_precedence_against_source() {
    let catalog = catalog(vec![spec(
        "g",
        r#"{"model":"openai/first"}"#,
        r#"{"id":"one","max_input_tokens":7}"#,
    )]);
    let mut case = Case::plain(
        Arc::clone(&catalog),
        FakeSource::answering(&[(
            "openai/first",
            r#"{"max_input_tokens":100,"max_tokens":50}"#,
        )]),
    );
    case.discovery.store(
        "one",
        record(&catalog, 0, limits(Some(9), Some(9), Some(9))),
    );
    let info = case
        .merged(
            Some(SuppliedDeployment::from_spec(&catalog.models[0])),
            "g",
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.get("max_input_tokens"), Some(&number(7)));
    assert_eq!(info.get("max_tokens"), Some(&number(9)));
    assert_eq!(info.get("max_output_tokens"), Some(&number(9)));
}
