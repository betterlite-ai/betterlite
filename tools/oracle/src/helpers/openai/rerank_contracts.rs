use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_error::facts::presence::Field;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankRequest, RerankResponse,
};
use betterlite_openai::api::rerank_contracts::wire::{
    RerankContract, RerankContractError, RerankRecordKind, decode_rerank_contracts,
};
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, OptionalRerankParams,
    Params, RerankLogging, RerankModelInfo, RerankProviderResponse, ResponseTransform,
    ReturnDocuments, get_optional_rerank_params,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Number, Value, json};
use thiserror::Error;

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
    Config(#[from] FixtureError),
    #[error("the record could not be decoded: {0}")]
    Decode(#[from] RerankContractError),
    #[error("the supplied response is not a rerank response")]
    Response,
}

#[derive(Debug, Error)]
enum FixtureError {
    #[error("the fixture mapper refused the parameters")]
    Refused,
}

#[derive(Default)]
struct Recording {
    calls: Mutex<Vec<Json>>,
}

impl BaseRerankConfig for Recording {
    type Error = FixtureError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        Ok(request.headers.clone())
    }

    fn get_supported_cohere_rerank_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        if request.query == "refuse" {
            return Err(FixtureError::Refused);
        }
        let recorded = record(&request);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        Ok(Params::from([("mapped".to_owned(), Json::Bool(true))]))
    }
}

fn object_of(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<IndexMap<_, _>>(),
    )
}

fn record(request: &MapCohereRerankParams<'_>) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        "non_default_params".to_owned(),
        object_of(request.non_default_params),
    );
    fields.insert("model".to_owned(), Json::Text(request.model.to_owned()));
    fields.insert("drop_params".to_owned(), Json::Bool(request.drop_params));
    fields.insert("query".to_owned(), Json::Text(request.query.to_owned()));
    fields.insert(
        "documents".to_owned(),
        Json::List(
            request
                .documents
                .iter()
                .map(RerankDocument::as_json)
                .collect(),
        ),
    );
    insert_optionals(&mut fields, request);
    Json::Object(fields)
}

fn insert_optionals(fields: &mut IndexMap<String, Json>, request: &MapCohereRerankParams<'_>) {
    fields.insert(
        "custom_llm_provider".to_owned(),
        request
            .custom_llm_provider
            .map_or(Json::Null, |provider| Json::Text(provider.to_owned())),
    );
    fields.insert(
        "top_n".to_owned(),
        request
            .top_n
            .map_or(Json::Null, |top_n| Json::Number(Number::from(top_n))),
    );
    fields.insert(
        "rank_fields".to_owned(),
        request.rank_fields.map_or(Json::Null, |names| {
            Json::List(
                names
                    .iter()
                    .map(|name| Json::Text(name.clone()))
                    .collect(),
            )
        }),
    );
    fields.insert(
        "return_documents".to_owned(),
        request.return_documents.map_or(Json::Null, Json::Bool),
    );
    fields.insert(
        "max_chunks_per_doc".to_owned(),
        request
            .max_chunks_per_doc
            .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
    );
    fields.insert(
        "max_tokens_per_doc".to_owned(),
        request
            .max_tokens_per_doc
            .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
    );
    fields.insert(
        "instruction".to_owned(),
        request
            .instruction
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
}

const fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    }
}

fn decode_request(payload: &Value) -> Option<RerankRequest> {
    let bytes = serde_json::to_vec(payload).ok()?;
    match decode_rerank_contracts(&bytes, RerankRecordKind::Request, limits()).ok()? {
        RerankContract::Request(record) => Some(record),
        RerankContract::Response(_) => None,
    }
}

fn decode_response(payload: &Value) -> Option<RerankResponse> {
    let bytes = serde_json::to_vec(payload).ok()?;
    match decode_rerank_contracts(&bytes, RerankRecordKind::Response, limits()).ok()? {
        RerankContract::Response(record) => Some(record),
        RerankContract::Request(_) => None,
    }
}

fn accessor(response: &RerankResponse, key: &str) -> Value {
    let item = response.item(key).map_or_else(
        |_unknown| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    );
    json!({
        "contains": response.contains(key),
        "item": item,
        "get": response.get(
            key,
            Json::List(vec![Json::Text(String::from("__absent__"))]),
        ),
    })
}

fn request_outcome(payload: &Value) -> Value {
    decode_request(payload).map_or(json!({"ok": false}), |record| {
        json!({
            "ok": true,
            "dump": serde_json::to_value(record).unwrap_or(Value::Null),
        })
    })
}

fn response_outcome(payload: &Value, keys: &[String]) -> Value {
    let Some(response) = decode_response(payload) else {
        return json!({"ok": false});
    };
    let accessors: BTreeMap<String, Value> = keys
        .iter()
        .map(|key| (key.clone(), accessor(&response, key)))
        .collect();
    json!({
        "ok": true,
        "dump": serde_json::to_value(&response).unwrap_or(Value::Null),
        "accessors": accessors,
    })
}

fn wire_records(params: &WireParams) -> Value {
    json!({
        "requests": params.requests.iter().map(request_outcome).collect::<Vec<_>>(),
        "responses": params
            .responses
            .iter()
            .map(|payload| response_outcome(payload, &params.accessor_keys))
            .collect::<Vec<_>>(),
    })
}

fn option_projection(params: &ProjectionParams) -> Result<Value, ContractError> {
    let mut reports = Vec::new();
    for call in &params.calls {
        let mut nondefault = call.nondefault.clone();
        let return_documents = match call.return_documents {
            Field::Missing => ReturnDocuments::Default,
            Field::Null => ReturnDocuments::Null,
            Field::Value(value) => ReturnDocuments::Value(value),
        };
        let config = Recording::default();
        let returned = get_optional_rerank_params(
            &config,
            OptionalRerankParams {
                non_default_params: nondefault.as_mut(),
                model: &call.model,
                drop_params: call.drop_params,
                query: &call.query,
                documents: &call.documents,
                custom_llm_provider: call.custom_llm_provider.as_deref(),
                top_n: call.top_n,
                rank_fields: call.rank_fields.as_deref(),
                return_documents,
                max_chunks_per_doc: call.max_chunks_per_doc,
                max_tokens_per_doc: call.max_tokens_per_doc,
                instruction: call.instruction.as_deref(),
            },
        )?;
        let mapper = config
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned();
        reports.push(json!({
            "mapper": mapper.unwrap_or(Json::Null),
            "nondefault_after": nondefault.as_ref().map_or(Json::Null, object_of),
            "returned": object_of(&returned),
        }));
    }
    Ok(json!({"calls": reports}))
}

fn default_response(config: &Recording, params: &DefaultsParams) -> Result<Value, ContractError> {
    let response = decode_response(&params.response).ok_or(ContractError::Response)?;
    let mut logging = RerankLogging::default();
    let raw_response = RerankProviderResponse {
        status_code: 200,
        headers: Headers::new(),
        body: Vec::new(),
    };
    let request_data = Params::new();
    let optional_params = Params::new();
    let litellm_params = Params::new();
    let transformed = config.transform_rerank_response(
        response,
        ResponseTransform {
            model: "rerank-model",
            raw_response: &raw_response,
            logging: &mut logging,
            api_key: None,
            request_data: &request_data,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
        },
    )?;
    Ok(serde_json::to_value(transformed).unwrap_or(Value::Null))
}

fn base_defaults(params: &DefaultsParams) -> Result<Value, ContractError> {
    let config = Recording::default();
    let urls = params
        .api_bases
        .iter()
        .map(|api_base| config.get_complete_url(api_base.as_deref(), "rerank-model", None))
        .collect::<Vec<_>>();
    let request =
        config.transform_rerank_request("rerank-model", &Params::new(), &Headers::new(), None)?;
    let raised = config.get_error_class(
        &params.error.message,
        params.error.status_code,
        params.error.headers.clone(),
    );
    Ok(json!({
        "urls": urls,
        "request": object_of(&request),
        "response": default_response(&config, params)?,
        "error": {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": raised.headers,
        },
    }))
}

fn decimal_number(decimal: Decimal) -> Value {
    let mut text = decimal.normalize().to_string();
    if !text.contains('.') && !text.contains('e') {
        text.push_str(".0");
    }
    text.parse::<Number>().map_or(Value::Null, Value::Number)
}

fn query_cost(params: &CostParams) -> Value {
    let config = Recording::default();
    let mut costs = Vec::new();
    for case in &params.cases {
        let (prompt, completion) = config.calculate_rerank_cost(
            &params.model,
            None,
            case.billed_units.as_ref(),
            case.model_info.as_ref(),
        );
        costs.push(json!([decimal_number(prompt), decimal_number(completion)]));
    }
    json!({"costs": costs})
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::WireRecords { params } => Ok(wire_records(params)),
            Input::OptionProjection { params } => option_projection(params),
            Input::BaseDefaults { params } => base_defaults(params),
            Input::QueryCost { params } => Ok(query_cost(params)),
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
