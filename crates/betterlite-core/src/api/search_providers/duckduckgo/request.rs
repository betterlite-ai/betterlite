use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

use super::DuckDuckGoSearchError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuckDuckGoSearchRequest {
    #[serde(rename = "_duckduckgo_params")]
    pub params: DuckDuckGoParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuckDuckGoParams {
    #[serde(rename = "q")]
    pub query: String,
    pub format: String,
    // the source copies max_results as given, and a value that is not an int leaves the limit unset
    #[serde(
        rename = "_max_results",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_results: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pretty: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_redirect: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_html: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_disambig: Option<i64>,
}

pub fn build_request(
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<DuckDuckGoSearchRequest, DuckDuckGoSearchError> {
    let joined_query = match query {
        SearchQuery::Single(text) => text.clone(),
        SearchQuery::Multiple(items) => items.join(" "),
    };
    Ok(DuckDuckGoSearchRequest {
        params: DuckDuckGoParams {
            query: joined_query,
            format: "json".to_owned(),
            max_results: optional_params
                .get("max_results")
                .filter(|value| !value.is_null())
                .cloned(),
            pretty: int_option(optional_params, "pretty")?,
            no_redirect: int_option(optional_params, "no_redirect")?,
            no_html: int_option(optional_params, "no_html")?,
            skip_disambig: int_option(optional_params, "skip_disambig")?,
        },
    })
}

fn int_option(
    optional_params: &Object,
    field: &'static str,
) -> Result<Option<i64>, DuckDuckGoSearchError> {
    let Some(value) = optional_params.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_i64()
        .map_or(Err(DuckDuckGoSearchError::OptionType { field }), |number| {
            Ok(Some(number))
        })
}

#[must_use]
pub fn search_url(
    api_base: Option<&str>,
    environment_base: &str,
    body: Option<&DuckDuckGoSearchRequest>,
) -> String {
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(environment_base);
    let mut url = base.to_owned();
    if let Some(request) = body {
        url.push_str("/?");
        url.push_str(&encode_params(&request.params));
    }
    url
}

#[must_use]
pub fn encode_params(params: &DuckDuckGoParams) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("q", &params.query);
    serializer.append_pair("format", &params.format);
    if let Some(value) = &params.max_results {
        if let JsonValue::Array(items) = value {
            for item in items {
                serializer.append_pair("_max_results", &wire_text(item));
            }
        } else {
            serializer.append_pair("_max_results", &wire_text(value));
        }
    }
    let flags = [
        ("pretty", params.pretty),
        ("no_redirect", params.no_redirect),
        ("no_html", params.no_html),
        ("skip_disambig", params.skip_disambig),
    ];
    for (name, value) in flags {
        if let Some(number) = value {
            serializer.append_pair(name, &number.to_string());
        }
    }
    let encoded = serializer.finish();
    // Python's quote_plus leaves `~` alone and encodes `*`, the reverse of form_urlencoded
    encoded.replace('*', "%2A").replace("%7E", "~")
}

fn wire_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Bool(true) => "True".to_owned(),
        JsonValue::Bool(false) => "False".to_owned(),
        JsonValue::Null => "None".to_owned(),
        JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
    }
}
