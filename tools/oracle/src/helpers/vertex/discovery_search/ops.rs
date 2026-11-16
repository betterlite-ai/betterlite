use std::collections::BTreeMap;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::vertex_discovery_search::authorization::get_complete_url;
use betterlite_vertex::vertex_discovery_search::error::VertexDiscoverySearchError;
use betterlite_vertex::vertex_discovery_search::options::VertexSearchOptions;
use betterlite_vertex::vertex_discovery_search::records::cost::calculate_vector_store_cost;
use betterlite_vertex::vertex_discovery_search::records::payload::transform_search_response;
use betterlite_vertex::vertex_discovery_search::records::search::{
    SearchRequest, transform_search_request,
};
use betterlite_vertex::vertex_rag_store::records::payload::Reply;
use betterlite_vertex::vertex_rag_store::records::search::SearchQuery;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const QUERY_COST_MODEL: &str = "vertex_ai/search_api";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    // boxed so both payload variants carry a pointer, well within the 3x variant_size_differences enforces
    Target(Box<TargetCall>),
    Search(Box<SearchCall>),
    Cost,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetCall {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    litellm_params: BTreeMap<String, Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCall {
    // the handler's store id, which the Discovery Engine request path never reads
    #[serde(rename = "vector_store_id")]
    _vector_store_id: String,
    query: SearchQuery,
    api_base: String,
    #[serde(default)]
    litellm_params: BTreeMap<String, Json>,
    #[serde(default)]
    optional_params: BTreeMap<String, Json>,
    #[serde(default)]
    extra_body: Option<BTreeMap<String, Json>>,
    #[serde(default)]
    response: Option<ReplyCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    status: u16,
    body: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let outcomes = input
        .calls
        .iter()
        .map(outcome)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"calls": outcomes}))
}

fn outcome(entry: &Call) -> Result<Value, Box<HelperError>> {
    match entry {
        Call::Target(call) => target_outcome(call),
        Call::Search(call) => search_outcome(call),
        Call::Cost => Ok(cost_outcome()),
    }
}

fn target_outcome(call: &TargetCall) -> Result<Value, Box<HelperError>> {
    let options = VertexSearchOptions::from_params(&call.litellm_params).map_err(helper_error)?;
    let url = get_complete_url(
        call.api_base.as_deref(),
        VertexParams {
            project: text(&call.litellm_params, "vertex_project"),
            ai_project: text(&call.litellm_params, "vertex_ai_project"),
            location: text(&call.litellm_params, "vertex_location"),
            ai_location: text(&call.litellm_params, "vertex_ai_location"),
            credentials: None,
            ai_credentials: None,
        },
        &VertexSettings::default(),
        &options,
    )
    .map_err(helper_error)?;
    Ok(json!({"url": url}))
}

fn search_outcome(call: &SearchCall) -> Result<Value, Box<HelperError>> {
    let mut logging = BTreeMap::new();
    let request = transform_search_request(
        &SearchRequest {
            query: &call.query,
            optional_params: &call.optional_params,
            extra_body: call.extra_body.as_ref(),
            api_base: &call.api_base,
            litellm_params: &call.litellm_params,
        },
        &mut logging,
    )
    .map_err(helper_error)?;
    let body = serde_json::to_value(&request.body).or_value_error()?;
    let Some(response) = &call.response else {
        return Ok(json!({"url": request.url, "body": body}));
    };
    let page = transform_search_response(&reply_of(response), &logging).map_err(helper_error)?;
    let results = page_value(&page)?;
    Ok(json!({"url": request.url, "body": body, "results": results}))
}

const fn reply_of(call: &ReplyCall) -> Reply<'_> {
    Reply {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    }
}

// scores stay strings so the six-decimal form matches LiteLLM's float formatting exactly
fn page_value(page: &VectorStoreSearchResponse) -> Result<Value, Box<HelperError>> {
    let mut value = serde_json::to_value(page).or_value_error()?;
    let Some(results) = page.data.as_ref() else {
        return Ok(value);
    };
    let Some(entries) = value.get_mut("data").and_then(Value::as_array_mut) else {
        return Ok(value);
    };
    for (entry, result) in entries.iter_mut().zip(results) {
        let Some(score) = result.score.as_ref() else {
            continue;
        };
        let Some(fields) = entry.as_object_mut() else {
            continue;
        };
        fields.insert("score".to_owned(), Value::String(decimal_text(score.0)));
    }
    Ok(value)
}

fn cost_outcome() -> Value {
    let catalog = BundledCatalog::default().get();
    let prices = catalog
        .entries
        .get(QUERY_COST_MODEL)
        .map(|entry| &entry.prices);
    let (input, output) = calculate_vector_store_cost(prices);
    json!({"input": decimal_text(input), "output": decimal_text(output)})
}

fn decimal_text(value: Decimal) -> String {
    let rounded = value.round_dp_with_strategy(6, RoundingStrategy::MidpointNearestEven);
    format!("{rounded:.6}")
}

fn text<'params>(params: &'params BTreeMap<String, Json>, name: &str) -> Option<&'params str> {
    let Some(Json::Text(value)) = params.get(name) else {
        return None;
    };
    Some(value.as_str())
}

fn helper_error(error: VertexDiscoverySearchError) -> Box<HelperError> {
    let message = error.to_string();
    let (class, status) = match error {
        VertexDiscoverySearchError::TargetSelecting { .. }
        | VertexDiscoverySearchError::UnsupportedFields { .. } => ("BadRequestError", Some(400)),
        VertexDiscoverySearchError::CreateUnsupported => ("NotImplementedError", Some(500)),
        VertexDiscoverySearchError::Provider(provider) => {
            ("BaseLLMException", Some(provider.status_code))
        }
        VertexDiscoverySearchError::ParamType(..)
        | VertexDiscoverySearchError::HeaderText(_)
        | VertexDiscoverySearchError::StoreRequired
        | VertexDiscoverySearchError::Segment(..)
        | VertexDiscoverySearchError::Url(_)
        | VertexDiscoverySearchError::Credentials(_)
        | VertexDiscoverySearchError::Options(_) => ("ValueError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
