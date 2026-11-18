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
    let mapped = Mapped {
        depth: depth(optional_params)?,
        location_name: location_name(optional_params)?,
        domain: domain(optional_params)?,
    };
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

fn keyword(query: &SearchQuery) -> String {
    match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(parts) => parts.first().cloned().unwrap_or_default(),
    }
}

// LiteLLM tests max_results for truthiness, so a zero value contributes no depth at all
fn depth(optional_params: &Object) -> Result<Option<i64>, DataForSeoSearchError> {
    let Some(value) = optional_params.get("max_results") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let number: i64 = serde_json::from_value(value.clone()).map_err(|_source| {
        DataForSeoSearchError::OptionInteger {
            field: "max_results",
        }
    })?;
    if number == 0 {
        return Ok(None);
    }
    Ok(Some(number.min(MAX_DEPTH)))
}

fn location_name(optional_params: &Object) -> Result<Option<String>, DataForSeoSearchError> {
    let Some(value) = optional_params.get("country") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let country: String = serde_json::from_value(value.clone())
        .map_err(|_source| DataForSeoSearchError::OptionText { field: "country" })?;
    if country.is_empty() {
        return Ok(None);
    }
    Ok(Some(country))
}

fn domain(optional_params: &Object) -> Result<Option<Vec<String>>, DataForSeoSearchError> {
    let Some(value) = optional_params.get("search_domain_filter") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let domains: Vec<String> = serde_json::from_value(value.clone()).map_err(|_source| {
        DataForSeoSearchError::OptionList {
            field: "search_domain_filter",
        }
    })?;
    if domains.is_empty() {
        return Ok(None);
    }
    Ok(Some(domains))
}
