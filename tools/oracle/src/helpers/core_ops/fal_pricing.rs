use std::collections::BTreeMap;

use betterlite_core::api::fal::pricing::{
    FalCostRequest, FalImageResponse, FalPricingError, cost_calculator, fal_ai_passthrough_cost,
};
use betterlite_core::cost::image_dispatch::options::OptionalParams;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::token_rates::ModelInfo;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    DeploymentZeroWins,
    ResolutionCaseSensitive,
    AreaTieSmaller,
    ReturnedDimensionsWin,
    MegapixelRoundUp,
    EmptyResponseNoLookup,
    PassthroughCountTypes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalculatorCase {
    #[serde(default)]
    images: Vec<IndexMap<String, Json>>,
    #[serde(default)]
    optional_params: OptionalParams,
    #[serde(default)]
    model_info: Option<ModelInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassthroughCase {
    model: String,
    #[serde(default)]
    request_body: OptionalParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    model: String,
    #[serde(default)]
    catalog: IndexMap<String, Json>,
    #[serde(default)]
    cases: Vec<CalculatorCase>,
    #[serde(default)]
    passthrough_cases: Vec<PassthroughCase>,
}

// The case's catalog stands in for litellm.model_cost, so a raw lookup is the key and an effective one is provider-qualified.
struct CaseLookup {
    entries: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for CaseLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let scoped =
            custom_llm_provider.and_then(|name| self.entries.get(&format!("{name}/{model}")));
        scoped.or_else(|| self.entries.get(model)).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        priced(self.effective_model_info(model, custom_llm_provider))
    }
}

fn priced(found: Option<PriceRecord>) -> Option<PricedModel> {
    found.map(|record| PricedModel {
        record,
        ..PricedModel::default()
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let bytes =
            serde_json::to_vec(&input.catalog).map_err(|error| helper_error(error.to_string()))?;
        let lookup = CaseLookup {
            entries: serde_json::from_slice(&bytes)
                .map_err(|error| helper_error(error.to_string()))?,
        };
        if matches!(input.operation, Operation::PassthroughCountTypes) {
            return costs_outcome(&input.passthrough_cases, &|case| {
                fal_ai_passthrough_cost(&case.model, &case.request_body, &lookup)
            });
        }
        let catalog = ModelCatalog::parse(&bytes, UnknownFields::Reject)
            .map_err(|error| helper_error(error.to_string()))?;
        costs_outcome(&input.cases, &|case| {
            let images: Vec<Option<&IndexMap<String, Json>>> =
                case.images.iter().map(Some).collect();
            let request = FalCostRequest {
                model: &input.model,
                image_response: FalImageResponse::Image(&images),
                optional_params: Some(&case.optional_params),
                model_info: case.model_info.as_ref(),
            };
            cost_calculator(&request, &catalog, &lookup).map(Some)
        })
    }
}

// Each case's price as a normalized decimal text, null where the passthrough has no price.
fn costs_outcome<T>(
    cases: &[T],
    price: &dyn Fn(&T) -> Result<Option<Decimal>, FalPricingError>,
) -> HelperResult {
    let mut costs = Vec::with_capacity(cases.len());
    for case in cases {
        let cost = price(case).map_err(|error| helper_error(error.to_string()))?;
        costs.push(cost.map(|value| value.normalize().to_string()));
    }
    Ok(serde_json::json!({ "costs": costs }))
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
