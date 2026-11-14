use std::collections::BTreeMap;

use betterlite_cost::search::error::SearchCostError;
use betterlite_cost::search::model_info::SearchModelInfo;
use betterlite_cost::search::params::SearchOptionalParams;
use betterlite_cost::search::{SearchPricingSource, search_provider_cost_per_query};
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;
use tokio::runtime::Builder;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    model_cost: BTreeMap<String, SearchModelInfo>,
    model: String,
    custom_llm_provider: Option<String>,
    number_of_queries: u64,
    #[serde(default)]
    optional_params: SearchOptionalParams,
}

struct Records(BTreeMap<String, SearchModelInfo>);

#[derive(Debug, Error)]
#[error("{0} isn't mapped yet")]
struct Unmapped(String);

impl SearchPricingSource for Records {
    type Error = Unmapped;

    async fn model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<SearchModelInfo, Unmapped> {
        self.0
            .get(model)
            .cloned()
            .ok_or_else(|| Unmapped(model.to_owned()))
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                message: error.to_string(),
                status: None,
                error_type: None,
                param: None,
                code: None,
            })
        })?;
        let records = Records(input.model_cost.clone());
        let (input_cost, output_cost) = runtime
            .block_on(search_provider_cost_per_query(
                &records,
                &input.model,
                input.custom_llm_provider.as_deref(),
                input.number_of_queries,
                &input.optional_params,
            ))
            .map_err(|error| {
                let class = match &error {
                    SearchCostError::ModelInfo { .. } => "ModelNotMappedError",
                    SearchCostError::Overflow => "OverflowError",
                };
                Box::new(HelperError {
                    class: class.to_owned(),
                    message: error.to_string(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                })
            })?;
        let [input_cost, output_cost] =
            [input_cost, output_cost].map(|cost| cost.round_dp(9).to_f64());
        let (Some(input_cost), Some(output_cost)) = (input_cost, output_cost) else {
            return Err(Box::new(HelperError {
                class: "OverflowError".to_owned(),
                message: "the cost doesn't fit a float".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
            }));
        };
        Ok(json!({"input_cost": input_cost, "output_cost": output_cost}))
    }
}
