use std::collections::BTreeMap;

use betterlite_core::api::fal::pricing::{
    FalCostRequest, FalImageResponse, cost_calculator, fal_ai_passthrough_cost,
};
use betterlite_core::cost::image_dispatch::options::OptionalParams;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::token_rates::ModelInfo;
use indexmap::IndexMap;
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
        custom_llm_provider
            .and_then(|provider| self.entries.get(&format!("{provider}/{model}")).cloned())
            .or_else(|| self.entries.get(model).cloned())
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        let record = self.effective_model_info(model, custom_llm_provider)?;
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        if matches!(input.operation, Operation::PassthroughCountTypes) {
            return passthrough_outcome(input);
        }
        calculator_outcome(input)
    }
}

fn calculator_outcome(input: &Input) -> HelperResult {
    let bytes =
        serde_json::to_vec(&input.catalog).map_err(|error| helper_error(error.to_string()))?;
    let catalog = ModelCatalog::parse(&bytes, UnknownFields::Reject)
        .map_err(|error| helper_error(error.to_string()))?;
    let lookup = CaseLookup {
        entries: serde_json::from_slice(&bytes).map_err(|error| helper_error(error.to_string()))?,
    };
    let mut costs = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        let images: Vec<Option<&IndexMap<String, Json>>> = case.images.iter().map(Some).collect();
        let request = FalCostRequest {
            model: &input.model,
            image_response: FalImageResponse::Image(&images),
            optional_params: Some(&case.optional_params),
            model_info: case.model_info.as_ref(),
        };
        let cost = cost_calculator(&request, &catalog, &lookup)
            .map_err(|error| helper_error(error.to_string()))?;
        costs.push(cost.normalize().to_string());
    }
    Ok(serde_json::json!({ "costs": costs }))
}

fn passthrough_outcome(input: &Input) -> HelperResult {
    let bytes =
        serde_json::to_vec(&input.catalog).map_err(|error| helper_error(error.to_string()))?;
    let lookup = CaseLookup {
        entries: serde_json::from_slice(&bytes).map_err(|error| helper_error(error.to_string()))?,
    };
    let mut costs = Vec::with_capacity(input.passthrough_cases.len());
    for case in &input.passthrough_cases {
        let cost = fal_ai_passthrough_cost(&case.model, &case.request_body, &lookup)
            .map_err(|error| helper_error(error.to_string()))?;
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
