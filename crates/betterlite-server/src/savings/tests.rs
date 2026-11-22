#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{JsonObject, ModelCostEntries, ModelRegistry};
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::savings_pricing::caching::{RequestPricingInput, request_savings_pricing};
use betterlite_models::savings_pricing::context::{ModelIdentity, SavingsPricingContext as _};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_model_info::cache::DiscoveryCache;
use rust_decimal::Decimal;

use crate::cost_estimate_route::testing::service_from;
use crate::state::cost_service::Slot;
use crate::state::shared::Shared;

use super::{Context, DeploymentPricing};

fn bundled_bytes() -> Vec<u8> {
    (BundledCatalog::default().asset)().expect("the bundled catalog is readable")
}

fn deployment_pricing(costs: &str) -> DeploymentPricing {
    let model_cost: JsonObject =
        serde_json::from_str(costs).expect("the fixture is a model cost map");
    DeploymentPricing {
        catalog: Arc::new(DeploymentCatalog::default()),
        discovery: DiscoveryCache::for_deployments(0),
        registry: ModelRegistry {
            model_cost,
            runtime_registered: ModelCostEntries::default(),
            provider_sets: ProviderModelSets::default(),
            keys: ModelCostKeys::default(),
            live_replay: None,
        },
    }
}

fn number(record: &PriceRecord, key: &str) -> Option<Decimal> {
    match record.get(key) {
        Some(Scalar::Number(value)) => parse_decimal_lexeme(&value.to_string()).ok(),
        Some(
            Scalar::Null | Scalar::Bool(_) | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_),
        )
        | None => None,
    }
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

#[tokio::test]
async fn deployment_price_reads_the_router_override() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = deployment_pricing(
        r#"{"dep-1": {"input_cost_per_token": 2.0, "litellm_provider": "openai"}}"#,
    );
    let context = Context {
        lookup: &service.lookup,
        deployments: Some(deployments),
    };
    let priced = context
        .deployment_price(Some("dep-1"), "gpt-4o")
        .await
        .expect("the deployment answers with prices");
    assert_eq!(
        number(&priced.record, "input_cost_per_token"),
        Some(decimal("2"))
    );
    assert_eq!(
        number(&priced.record, "output_cost_per_token"),
        Some(decimal("0.00001"))
    );
}

#[tokio::test]
async fn deployment_price_falls_back_to_the_model_name() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = DeploymentPricing::default();
    let context = Context {
        lookup: &service.lookup,
        deployments: Some(deployments),
    };
    let priced = context
        .deployment_price(Some("no-such-deployment"), "gpt-4o")
        .await
        .expect("the model name stands in for the deployment");
    assert_eq!(
        number(&priced.record, "input_cost_per_token"),
        Some(decimal("0.0000025"))
    );
}

#[tokio::test]
async fn deployment_info_reads_the_public_model_info() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = DeploymentPricing::default();
    let info = deployments
        .deployment_info(&service.lookup, "no-such-deployment", "gpt-4o")
        .await
        .expect("the public entry stands in");
    assert!(matches!(
        info.get("input_cost_per_token"),
        Some(Json::Number(_))
    ));
}

#[tokio::test]
async fn deployment_price_keeps_the_deployment_tables() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = deployment_pricing(
        r#"{"dep-t": {"input_cost_per_token": 1.0,
            "tiered_pricing": [{"input_cost_per_token": 0.5, "range": [0, 1000]}],
            "off_peak_pricing": {"input_cost_per_token": 0.25, "cache_read_input_token_cost": 0.1}}}"#,
    );
    let context = Context {
        lookup: &service.lookup,
        deployments: Some(deployments),
    };
    let priced = context
        .deployment_price(Some("dep-t"), "gpt-4o")
        .await
        .expect("the deployment answers with its tables");
    assert!(matches!(
        priced
            .tiered_pricing
            .first()
            .and_then(|tier| tier.input_token.as_ref()),
        Some(Scalar::Number(_))
    ));
    let block = priced
        .off_peak_pricing
        .as_ref()
        .expect("the off-peak block is kept");
    assert!(block.input_cost_per_token.is_some());
    assert!(priced.off_peak_cache_read_present);
}

#[tokio::test]
async fn deployment_price_without_an_id_has_no_deployment() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = DeploymentPricing::default();
    let context = Context {
        lookup: &service.lookup,
        deployments: Some(deployments),
    };
    assert!(context.deployment_price(None, "gpt-4o").await.is_none());
}

#[tokio::test]
async fn deployment_price_without_a_resolver_is_unavailable() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let context = Context {
        lookup: &service.lookup,
        deployments: None,
    };
    assert!(
        context
            .deployment_price(Some("dep-1"), "gpt-4o")
            .await
            .is_none()
    );
}

#[test]
fn context_resolves_the_canonical_identity() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = DeploymentPricing::default();
    let context = Context {
        lookup: &service.lookup,
        deployments: Some(deployments),
    };
    let identity = context
        .resolve_model("anthropic/claude-3-haiku-20240307", None)
        .expect("the prefixed name resolves");
    assert_eq!(
        identity,
        ModelIdentity {
            model: String::from("claude-3-haiku-20240307"),
            provider: String::from("anthropic"),
        }
    );
}

#[test]
fn context_keeps_the_custom_provider_decision() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = DeploymentPricing::default();
    let context = Context {
        lookup: &service.lookup,
        deployments: Some(deployments),
    };
    let identity = context
        .resolve_model("gpt-4o", Some("openai"))
        .expect("the supplied provider resolves");
    assert_eq!(identity.provider, "openai");
    assert_eq!(identity.model, "gpt-4o");
}

#[tokio::test]
async fn request_prices_fall_back_to_the_public_lookup() {
    let bytes = bundled_bytes();
    let service = service_from(&bytes).expect("the cost service builds");
    let deployments = DeploymentPricing::default();
    let context = Context {
        lookup: &service.lookup,
        deployments: Some(deployments),
    };
    let pricing = request_savings_pricing(
        &context,
        &RequestPricingInput {
            model: Some("openai/gpt-4o"),
            custom_llm_provider: None,
            model_id: None,
        },
    )
    .await;
    assert_eq!(pricing.provider.as_deref(), Some("openai"));
    assert_eq!(
        pricing
            .pricing
            .as_ref()
            .and_then(|priced| number(&priced.record, "input_cost_per_token")),
        Some(decimal("0.0000025"))
    );
}

#[tokio::test]
async fn the_cost_resource_carries_the_savings_context() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("model_list:\n  - {model_name: g, litellm_params: {model: openai/gpt-4o, api_key: k}}\n")
    .expect("the config loads");
    let mut bundled = BundledCatalog::default();
    let shared = Shared {
        bundled_catalog: bundled.get(),
        client: transport(),
    };
    let slot = Slot::new(&config, &shared);
    let context = slot
        .savings()
        .expect("the built cost service carries a savings context");
    assert!(
        context
            .deployment_price(Some("dep-1"), "gpt-4o")
            .await
            .is_none()
    );
    let pricing = request_savings_pricing(
        &context,
        &RequestPricingInput {
            model: Some("openai/gpt-4o"),
            custom_llm_provider: None,
            model_id: None,
        },
    )
    .await;
    assert_eq!(pricing.provider.as_deref(), Some("openai"));
    assert!(pricing.pricing.is_some());
}

fn transport() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}
