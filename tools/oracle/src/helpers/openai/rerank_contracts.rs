use std::collections::BTreeMap;

use betterlite_error::facts::presence::Field;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankBilledUnits, RerankDocument};
use betterlite_provider::api::rerank_contracts::RerankModelInfo;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

mod defaults;
mod projection;
mod query_cost;
mod recording;
mod wire_records;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    WireRecords { params: WireParams },
    OptionProjection { params: ProjectionParams },
    BaseDefaults { params: DefaultsParams },
    QueryCost { params: CostParams },
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireParams {
    requests: Vec<Value>,
    responses: Vec<Value>,
    accessor_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProjectionParams {
    calls: Vec<ProjectionCall>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProjectionCall {
    model: String,
    drop_params: bool,
    query: String,
    documents: Vec<RerankDocument>,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    top_n: Option<i64>,
    #[serde(default)]
    rank_fields: Option<Vec<String>>,
    #[serde(default)]
    return_documents: Field<bool>,
    #[serde(default)]
    max_chunks_per_doc: Option<i64>,
    #[serde(default)]
    max_tokens_per_doc: Option<i64>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    nondefault: Option<BTreeMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DefaultsParams {
    api_bases: Vec<Option<String>>,
    response: Value,
    error: ErrorParams,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorParams {
    message: String,
    status_code: u16,
    headers: BTreeMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CostParams {
    model: String,
    cases: Vec<CostCase>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CostCase {
    #[serde(default)]
    model_info: Option<RerankModelInfo>,
    #[serde(default)]
    billed_units: Option<RerankBilledUnits>,
}

#[derive(Debug, Error)]
enum ContractError {
    #[error("the fixture config refused the parameters: {0}")]
    Config(#[from] recording::FixtureError),
    #[error("the supplied response is not a rerank response")]
    Response,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::WireRecords { params } => Ok(wire_records::run(params)),
            Input::OptionProjection { params } => projection::run(params),
            Input::BaseDefaults { params } => defaults::run(params),
            Input::QueryCost { params } => Ok(query_cost::run(params)),
        };
        output.map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
