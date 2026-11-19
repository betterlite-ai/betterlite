use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::super::google_pse::wire::{self, SearchParam, WireError};
use super::BraveSearchError;

const METADATA_PARAM: &str = "include_fetch_metadata";
const MAX_RESULTS_PARAM: &str = "max_results";
const DOMAIN_FILTER: &str = "search_domain_filter";
const COUNT_PARAM: &str = "count";
const QUERY_PARAM: &str = "q";
const COUNT_LIMIT: i64 = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BraveBody {
    #[serde(rename = "_brave_params")]
    pub params: IndexMap<String, BraveParam>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BraveParam {
    Text(String),
    Number(i64),
    Bool(bool),
    Many(Vec<BraveParam>),
}

impl BraveParam {
    // the source urlencodes with doseq, where Python writes True and None by name
    fn wire(&self) -> SearchParam {
        match self {
            Self::Text(text) => SearchParam::Text(text.clone()),
            Self::Number(number) => SearchParam::Number(*number),
            Self::Bool(flag) => SearchParam::Text(if *flag { "True" } else { "False" }.to_owned()),
            Self::Many(items) => SearchParam::Many(items.iter().map(Self::wire).collect()),
        }
    }
}

pub(super) fn build(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<BraveBody, BraveSearchError> {
    let mut params = IndexMap::new();
    params.insert(QUERY_PARAM.to_owned(), BraveParam::Text(join_query(query)));
    let metadata = optional_params
        .get(METADATA_PARAM)
        .and_then(|value| value.as_bool())
        != Some(false);
    params.insert(METADATA_PARAM.to_owned(), BraveParam::Bool(metadata));
    apply_count(&mut params, optional_params)?;
    apply_domains(&mut params, optional_params)?;
    apply_passthrough(&mut params, optional_params)?;
    Ok(BraveBody { params })
}

pub(super) fn query_string(params: &IndexMap<String, BraveParam>) -> Result<String, WireError> {
    let mut encoded: IndexMap<String, SearchParam> = IndexMap::with_capacity(params.len());
    for (name, param) in params {
        encoded.insert(name.clone(), param.wire());
    }
    wire::query_string(&encoded, true)
}

fn join_query(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    }
}

// the source caps max_results above only; a zero or negative count reaches the API as given
fn apply_count(
    params: &mut IndexMap<String, BraveParam>,
    optional_params: &Object,
) -> Result<(), BraveSearchError> {
    let Some(value) = optional_params
        .get(MAX_RESULTS_PARAM)
        .filter(|value| !value.is_null())
    else {
        return Ok(());
    };
    let count = value.as_i64().ok_or_else(|| BraveSearchError::OptionType {
        field: MAX_RESULTS_PARAM.to_owned(),
    })?;
    params.insert(
        COUNT_PARAM.to_owned(),
        BraveParam::Number(count.min(COUNT_LIMIT)),
    );
    Ok(())
}

fn apply_domains(
    params: &mut IndexMap<String, BraveParam>,
    optional_params: &Object,
) -> Result<(), BraveSearchError> {
    let Some(domains) = optional_params
        .get(DOMAIN_FILTER)
        .and_then(|value| value.as_array())
        .filter(|items| !items.is_empty())
    else {
        return Ok(());
    };
    let mut clauses = Vec::with_capacity(domains.len());
    for domain in domains {
        let text = domain
            .as_str()
            .ok_or_else(|| BraveSearchError::OptionType {
                field: DOMAIN_FILTER.to_owned(),
            })?;
        clauses.push(format!("site:{text}"));
    }
    let Some(BraveParam::Text(query)) = params.get(QUERY_PARAM) else {
        return Ok(());
    };
    let filtered = format!("({query}) AND ({})", clauses.join(" OR "));
    params.insert(QUERY_PARAM.to_owned(), BraveParam::Text(filtered));
    Ok(())
}

fn apply_passthrough(
    params: &mut IndexMap<String, BraveParam>,
    optional_params: &Object,
) -> Result<(), BraveSearchError> {
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str())
            || params.contains_key(name)
            || value.is_null()
        {
            continue;
        }
        let param: BraveParam = serde_json::from_value(value.clone()).map_err(|_error| {
            BraveSearchError::OptionType {
                field: name.clone(),
            }
        })?;
        params.insert(name.clone(), param);
    }
    Ok(())
}
