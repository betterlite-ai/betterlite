use betterlite_models::catalog::CatalogEntry;
use betterlite_models::entry::ModelFields;
use betterlite_models::price_fields::PriceFields;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_vertex::google_pricing::character::{CharacterRequest, cost_per_character};
use betterlite_vertex::google_pricing::delegate::{GenericTokenPricing, GenericTokenRequest};
use betterlite_vertex::google_pricing::error::GooglePricingError;
use betterlite_vertex::google_pricing::grounding::{
    cost_per_web_search_request, maps_grounding_cost,
};
use betterlite_vertex::google_pricing::routing::{CostMetric, cost_router};
use betterlite_vertex::google_pricing::token::{TokenRequest, cost_per_token};
use rust_decimal::Decimal;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::helpers::HelperError;

use super::input::{Input, Operation};

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    match input.operation {
        Operation::CostRouter => Ok(router(input)),
        Operation::CostPerCharacter | Operation::CostPerToken => priced(input),
        Operation::WebSearchCost => search(input),
        Operation::MapsGroundingCost => maps(input),
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "GooglePricingError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn catalog_entry(name: &str, raw: &RawValue) -> Result<CatalogEntry, Box<HelperError>> {
    let (fields, _unknown) = ModelFields::parse(raw).map_err(|error| failure(error.to_string()))?;
    let prices = PriceFields::parse(raw).map_err(|error| failure(error.to_string()))?;
    Ok(CatalogEntry {
        name: name.to_owned(),
        fields,
        prices,
    })
}

fn required_record(input: &Input) -> Result<CatalogEntry, Box<HelperError>> {
    let raw = input
        .record
        .as_ref()
        .ok_or_else(|| failure("record is required".to_owned()))?;
    catalog_entry(&input.model, raw)
}

fn router(input: &Input) -> Value {
    let metric = match cost_router(&input.model, &input.custom_llm_provider, &input.call_type) {
        CostMetric::PerCharacter => "cost_per_character",
        CostMetric::PerToken => "cost_per_token",
    };
    json!({ "metric": metric })
}

// Stands in for B068's shared calculator: bills the token counts it receives and resolves the regional uplift.
struct FixturePricing;

impl GenericTokenPricing for FixturePricing {
    fn generic_cost_per_token(
        &self,
        request: &GenericTokenRequest<'_>,
    ) -> Result<GenericCostResult, TokenCostError> {
        Ok(GenericCostResult {
            prompt_cost: Decimal::from(request.usage.prompt_tokens),
            completion_cost: Decimal::from(request.usage.completion_tokens),
        })
    }

    fn vertex_endpoint_uplift(
        &self,
        record: &CatalogEntry,
        vertex_location: Option<&str>,
    ) -> Decimal {
        match vertex_location {
            None => Decimal::ONE,
            Some(location) if location.eq_ignore_ascii_case("global") => Decimal::ONE,
            Some(_) => record
                .prices
                .values
                .other
                .regional_endpoint_uplift
                .unwrap_or(Decimal::ONE),
        }
    }
}

fn cost_value(
    result: Result<GenericCostResult, GooglePricingError>,
) -> Result<Value, Box<HelperError>> {
    let cost = result.map_err(|error| failure(error.to_string()))?;
    Ok(json!({
        "prompt_cost": cost.prompt_cost.normalize().to_string(),
        "completion_cost": cost.completion_cost.normalize().to_string(),
    }))
}

fn priced(input: &Input) -> Result<Value, Box<HelperError>> {
    let record = required_record(input)?;
    let usage = input.usage.google_usage();
    let pricing = FixturePricing;
    let cost = match input.operation {
        Operation::CostPerCharacter => cost_per_character(
            &pricing,
            &record,
            &usage,
            &CharacterRequest {
                model: &input.model,
                custom_llm_provider: &input.custom_llm_provider,
                prompt_characters: input.prompt_characters,
                completion_characters: input.completion_characters,
                service_tier: input.service_tier.as_deref(),
                vertex_location: input.vertex_location.as_deref(),
            },
        ),
        Operation::CostPerToken => cost_per_token(
            &pricing,
            &record,
            &usage,
            &TokenRequest {
                model: &input.model,
                custom_llm_provider: &input.custom_llm_provider,
                service_tier: input.service_tier.as_deref(),
                vertex_location: input.vertex_location.as_deref(),
            },
        ),
        Operation::CostRouter | Operation::WebSearchCost | Operation::MapsGroundingCost => {
            return Err(failure(
                "the operation does not bill a token or character cost".to_owned(),
            ));
        }
    };
    cost_value(cost)
}

fn search(input: &Input) -> Result<Value, Box<HelperError>> {
    let record = required_record(input)?;
    let usage = input.usage.google_usage();
    let cost =
        cost_per_web_search_request(&usage, &record).map_err(|error| failure(error.to_string()))?;
    Ok(json!({ "cost": cost.normalize().to_string() }))
}

fn maps(input: &Input) -> Result<Value, Box<HelperError>> {
    let record = required_record(input)?;
    let usage = input.usage.google_usage();
    let cost = maps_grounding_cost(&usage, &record).map_err(|error| failure(error.to_string()))?;
    Ok(json!({ "cost": cost.normalize().to_string() }))
}
