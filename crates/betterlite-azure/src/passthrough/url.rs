use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_models::decisions::Json;
use betterlite_openai::passthrough::transformation::types::PassthroughError;
use serde::Serialize;

use crate::azure_urls::AzureUrlError;
use crate::azure_urls::url_parts::{encoded_query, query_pairs};

pub(crate) const API_VERSION: &str = "api-version";
// litellm_params carries the same deployment version under its own underscore key
pub(crate) const PARAMS_API_VERSION: &str = "api_version";

// Python truthiness of a JSON value: bool(request_data.get("stream")) and the api-version checks read 0, "", false, [] and {} as empty
pub(crate) fn is_filled(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_none_or(|magnitude| magnitude != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

// httpx.URL.copy_with(params=QueryParams(kept)): duplicate names regroup by first-occurrence key order
pub fn without_api_version(api_base: &str) -> Result<String, AzureUrlError> {
    let mut url = HttpxUrl::parse(api_base).map_err(AzureUrlError::InvalidUrl)?;
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for (name, value) in query_pairs(url.query.as_deref()) {
        if name == API_VERSION {
            continue;
        }
        match grouped
            .iter_mut()
            .find(|(kept_name, _values)| *kept_name == name)
        {
            Some((_kept_name, values)) => values.push(value),
            None => grouped.push((name, vec![value])),
        }
    }
    let kept: Vec<(String, String)> = grouped
        .into_iter()
        .flat_map(|(name, values)| values.into_iter().map(move |value| (name.clone(), value)))
        .collect();
    url.query = encoded_query(&kept);
    Ok(url.href())
}

fn version_text(value: &Json) -> Result<String, PassthroughError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Number(number) => Ok(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            Err(PassthroughError::QueryValue {
                name: API_VERSION.to_owned(),
            })
        }
    }
}

pub(crate) fn version_from<Raw>(raw: Option<&Raw>) -> Result<Option<String>, PassthroughError>
where
    Raw: Serialize,
{
    let Some(value) = raw
        .and_then(|supplied| serde_json::to_value(supplied).ok())
        .and_then(|converted| serde_json::from_value::<Json>(converted).ok())
        .filter(is_filled)
    else {
        return Ok(None);
    };
    version_text(&value).map(Some)
}
