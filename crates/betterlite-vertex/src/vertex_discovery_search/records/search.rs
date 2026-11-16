use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::error::VertexDiscoverySearchError;
use super::super::options::VertexSearchOptions;
use crate::vertex_rag_store::records::search::SearchQuery;

const SEARCH_SUFFIX: &str = ":search";
const DEFAULT_PAGE_SIZE: i64 = 10;

// B1058's native SearchRequest fields, as named in its two extra_body TypedDicts
const DATASTORE_FIELDS: [&str; 30] = [
    "query",
    "pageSize",
    "pageToken",
    "offset",
    "oneBoxPageSize",
    "pageCategories",
    "imageQuery",
    "filter",
    "canonicalFilter",
    "orderBy",
    "userInfo",
    "languageCode",
    "facetSpecs",
    "boostSpec",
    "params",
    "queryExpansionSpec",
    "spellCorrectionSpec",
    "userPseudoId",
    "contentSearchSpec",
    "rankingExpression",
    "rankingExpressionBackend",
    "safeSearch",
    "userLabels",
    "naturalLanguageQueryUnderstandingSpec",
    "searchAsYouTypeSpec",
    "displaySpec",
    "crowdingSpecs",
    "relevanceThreshold",
    "relevanceScoreSpec",
    "customRankingParams",
];
const ENGINE_ONLY_FIELDS: [&str; 2] = ["dataStoreSpecs", "numResultsPerDataStore"];
const TARGET_SELECTING_FIELDS: [&str; 3] = ["branch", "entity", "servingConfig"];

pub struct SearchRequest<'request> {
    pub query: &'request SearchQuery,
    pub optional_params: &'request BTreeMap<String, Json>,
    pub extra_body: Option<&'request BTreeMap<String, Json>>,
    pub api_base: &'request str,
    pub litellm_params: &'request BTreeMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCall {
    pub url: String,
    pub body: Json,
}

// LiteLLM's transform_search_vector_store_request: the merged query is logged for the response transform
pub fn transform_search_request(
    request: &SearchRequest<'_>,
    logging: &mut BTreeMap<String, Json>,
) -> Result<SearchCall, VertexDiscoverySearchError> {
    let options = VertexSearchOptions::from_params(request.litellm_params)?;
    let mut body = IndexMap::from([
        ("query".to_owned(), Json::Text(request.query.joined())),
        (
            "pageSize".to_owned(),
            Json::Number(Number::from(DEFAULT_PAGE_SIZE)),
        ),
    ]);
    if let Some(page_size) = page_size(request.optional_params)? {
        body.insert("pageSize".to_owned(), page_size);
    }
    if let Some(extra_body) = request.extra_body {
        body.extend(filter_extra_body(
            extra_body,
            options.vertex_engine_id.is_some(),
        )?);
    }
    let query = merged_query(&body)?;
    logging.insert("query".to_owned(), Json::Text(query.to_owned()));
    let mut url = request.api_base.to_owned();
    url.push_str(SEARCH_SUFFIX);
    Ok(SearchCall {
        url,
        body: Json::Object(body),
    })
}

// an explicit zero max_num_results is kept, while an absent or null one leaves the default page size
fn page_size(
    params: &BTreeMap<String, Json>,
) -> Result<Option<Json>, VertexDiscoverySearchError> {
    match params.get("max_num_results") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => match number.as_i64() {
            Some(value) => Ok(Some(Json::Number(Number::from(value)))),
            None => Err(VertexDiscoverySearchError::ParamType(
                "max_num_results",
                "an integer",
            )),
        },
        Some(_) => Err(VertexDiscoverySearchError::ParamType(
            "max_num_results",
            "an integer",
        )),
    }
}

// LiteLLM's _filter_extra_body: target-selecting fields fail before unknown ones, and None values never count
fn filter_extra_body(
    extra_body: &BTreeMap<String, Json>,
    is_engine: bool,
) -> Result<IndexMap<String, Json>, VertexDiscoverySearchError> {
    let present = extra_body
        .iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .map(|(key, value)| (key.as_str(), value))
        .collect::<Vec<_>>();
    let mut targeting = present
        .iter()
        .filter(|(key, _)| TARGET_SELECTING_FIELDS.contains(key))
        .map(|(key, _)| (*key).to_owned())
        .collect::<Vec<_>>();
    if !targeting.is_empty() {
        targeting.sort_unstable();
        return Err(VertexDiscoverySearchError::TargetSelecting { fields: targeting });
    }
    let supported = supported_fields(is_engine);
    let mut unsupported = present
        .iter()
        .filter(|(key, _)| !supported.contains(key))
        .map(|(key, _)| (*key).to_owned())
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        unsupported.sort_unstable();
        return Err(VertexDiscoverySearchError::UnsupportedFields {
            mode: if is_engine { "engine/app" } else { "data store" },
            fields: unsupported,
            supported: supported
                .iter()
                .map(|field| (*field).to_owned())
                .collect(),
        });
    }
    Ok(present
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect())
}

fn supported_fields(is_engine: bool) -> Vec<&'static str> {
    let mut fields = DATASTORE_FIELDS.to_vec();
    if is_engine {
        fields.extend_from_slice(&ENGINE_ONLY_FIELDS);
    }
    fields.sort_unstable();
    fields
}

fn merged_query(body: &IndexMap<String, Json>) -> Result<&str, VertexDiscoverySearchError> {
    match body.get("query") {
        Some(Json::Text(text)) => Ok(text),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VertexDiscoverySearchError::ParamType("query", "a string"))
        }
    }
}
