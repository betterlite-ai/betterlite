use std::convert::Infallible;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{ProviderIdentity, ResolutionContext, ResolvedModel};
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;

use super::super::billing::{FailurePrice as _, NativePrice};
use super::super::events::nested_response;
use super::support::completed_event;
use crate::cost::call_price::policy::ProviderPricePolicy;
use crate::cost::completion::args::{CostCalculator, SpecialBorrows, ToolBorrows};
use crate::cost::completion::tests::rig::{Rig, args, rig};
use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::policy::{ImagePricePolicy, ImageRoute};
use crate::cost::prepare::{BillingTokenCounter, ResponseView, TypedResponse};
use crate::cost::special::policy::{ModelResolver, SpecialPricePolicy};
use crate::cost::usage::{BillingUsage, ResponseKind};

struct Policy;

impl ModelResolver for Policy {
    type Failure = &'static str;
    fn resolve_model(&self, _model: &str) -> Result<ResolvedModel, &'static str> {
        Err("explicit model and provider avoid resolution")
    }
}

impl ImagePricePolicy for Policy {
    type Error = Infallible;
    fn image_cost(&self, _route: &ImageRoute<'_>) -> Result<Decimal, Infallible> {
        Ok(Decimal::ZERO)
    }
}

impl ProviderInference for Policy {
    type Failure = Infallible;
    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, Infallible> {
        Ok(ProviderIdentity::Custom("openai".to_owned()))
    }
}

impl ProviderPricePolicy for Policy {}
impl SpecialPricePolicy for Policy {}

#[async_trait]
impl BillingTokenCounter for Policy {
    async fn count_messages(&self, _model: &str, _messages: &[Json]) -> Result<usize, CountError> {
        panic!("the failed response already carries usage")
    }
    async fn count_text(&self, _model: &str, _text: &str) -> Result<usize, CountError> {
        panic!("the failed response already carries usage")
    }
}

fn calculator<'rig>(
    fixture: &'rig Rig,
    resolution: &'rig ResolutionContext<'rig>,
) -> CostCalculator<'rig, Policy, Policy, Policy> {
    CostCalculator {
        special: SpecialBorrows {
            lookup: &fixture.catalog,
            resolver: &Policy,
            image_policy: &Policy,
            special_policy: &Policy,
            size_pattern: &fixture.pattern,
            database: &fixture.database,
            replicate_models: &fixture.models,
            replicate_price: Decimal::ZERO,
            discount: &fixture.discount,
            margin: &fixture.margin,
        },
        catalog: &fixture.catalog,
        inference: &Policy,
        counter: &Policy,
        price_policy: &Policy,
        resolution,
        tools: ToolBorrows {
            source: &fixture.tools,
            policy: &fixture.tools,
            prices: &ToolPrices::PINNED,
        },
    }
}

#[tokio::test]
async fn native_failure_billing_uses_completion_cost() {
    let fixture = rig(&[(
        "gpt-4o",
        &[
            ("input_cost_per_token", "0.001"),
            ("output_cost_per_token", "0.002"),
        ],
    )]);
    let resolution = ResolutionContext {
        catalog: &fixture.catalog_model,
        known_model_names: &fixture.names,
        custom_providers: &fixture.names,
        environment: &fixture.environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let services = calculator(&fixture, &resolution);
    let event = completed_event();
    let response = nested_response(&event).unwrap();
    let view = ResponseView::Typed(TypedResponse {
        kind: ResponseKind::Responses,
        model: Some("gpt-4o"),
        usage: BillingUsage::Response(response.usage.as_ref().unwrap()),
        service_tier: None,
        hidden_params: None,
    });
    let charge = args(Some(&view));
    let price = NativePrice {
        services: &services,
        args: &charge,
        moment: UNIX_EPOCH,
    };
    assert_eq!(price.cost(response).await, Some(Decimal::new(2, 2)));
}
