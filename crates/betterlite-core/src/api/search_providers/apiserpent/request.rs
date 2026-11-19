use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;

use super::params::ApiserpentParams;
use super::{ApiserpentBody, ApiserpentSearchError};
use crate::api::responses_input_content::is_truthy;

pub(super) const DEEP_PARAM: &str = "deep";
const DOMAIN_FILTER: &str = "search_domain_filter";

pub(super) fn build(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<ApiserpentBody, ApiserpentSearchError> {
    let deep = is_truthy(optional_params.get(DEEP_PARAM));
    let params = ApiserpentParams::from_optional(optional_params, deep)?;
    let joined = join_query(query);
    let text = match domain_clauses(optional_params)? {
        Some(clauses) => format!("({joined}) ({})", clauses.join(" OR ")),
        None => joined,
    };
    Ok(ApiserpentBody {
        params: params.to_request_params(text),
    })
}

fn join_query(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    }
}

fn domain_clauses(optional_params: &Object) -> Result<Option<Vec<String>>, ApiserpentSearchError> {
    let Some(domains) = optional_params
        .get(DOMAIN_FILTER)
        .and_then(|value| value.as_array())
        .filter(|domains| !domains.is_empty())
    else {
        return Ok(None);
    };
    let clauses: Result<Vec<String>, ApiserpentSearchError> = domains
        .iter()
        .map(|domain| {
            domain.as_str().map(|text| format!("site:{text}")).ok_or(
                ApiserpentSearchError::OptionList {
                    field: DOMAIN_FILTER,
                },
            )
        })
        .collect();
    clauses.map(Some)
}
