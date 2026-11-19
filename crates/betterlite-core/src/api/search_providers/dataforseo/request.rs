use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{PERPLEXITY_PARAMS, SearchQuery};

use super::{DataForSeoSearchError, DataForSeoSearchRequest, DataForSeoSearchTask};

const MAX_DEPTH: i64 = 700;

struct Mapped {
    depth: Option<i64>,
    location_name: Option<String>,
    domain: Option<Vec<String>>,
}

impl Mapped {
    // the names the task object already holds, so a native parameter under one of them never replaces it
    fn holds(&self, name: &str) -> bool {
        match name {
            "keyword" | "language_code" => true,
            "depth" => self.depth.is_some(),
            "location_name" => self.location_name.is_some(),
            "domain" => self.domain.is_some(),
            "location_code" => self.location_name.is_none(),
            _ => false,
        }
    }
}

pub(super) fn build(
    query: &SearchQuery,
    optional_params: &Object,
    location_code: i64,
) -> Result<DataForSeoSearchRequest, DataForSeoSearchError> {
    let mapped = map_options(optional_params)?;
    let default_location = mapped.location_name.is_none().then_some(location_code);
    let mut extra = Object::new();
    for (name, value) in optional_params {
        if PERPLEXITY_PARAMS.contains(&name.as_str()) || mapped.holds(name) {
            continue;
        }
        extra.insert(name.clone(), value.clone());
    }
    let task = DataForSeoSearchTask {
        keyword: keyword(query),
        depth: mapped.depth,
        location_name: mapped.location_name,
        domain: mapped.domain,
        language_code: String::from("en"),
        location_code: default_location,
        extra,
    };
    Ok(DataForSeoSearchRequest(vec![task]))
}

// the unified options LiteLLM maps into the task, read only when present and not null
fn map_options(optional_params: &Object) -> Result<Mapped, DataForSeoSearchError> {
    let mut mapped = Mapped {
        depth: None,
        location_name: None,
        domain: None,
    };
    if let Some(value) = optional_params
        .get("max_results")
        .filter(|value| !value.is_null())
    {
        let number: i64 = serde_json::from_value(value.clone()).map_err(|_source| {
            DataForSeoSearchError::OptionInteger {
                field: "max_results",
            }
        })?;
        mapped.depth = (number != 0).then_some(number.min(MAX_DEPTH));
    }
    if let Some(value) = optional_params
        .get("country")
        .filter(|value| !value.is_null())
    {
        let country: String = serde_json::from_value(value.clone())
            .map_err(|_source| DataForSeoSearchError::OptionText { field: "country" })?;
        mapped.location_name = (!country.is_empty()).then_some(country);
    }
    if let Some(value) = optional_params
        .get("search_domain_filter")
        .filter(|value| !value.is_null())
    {
        let domains: Vec<String> = serde_json::from_value(value.clone()).map_err(|_source| {
            DataForSeoSearchError::OptionList {
                field: "search_domain_filter",
            }
        })?;
        mapped.domain = (!domains.is_empty()).then_some(domains);
    }
    Ok(mapped)
}

fn keyword(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(parts) => parts.first().cloned().unwrap_or_default(),
    }
}
