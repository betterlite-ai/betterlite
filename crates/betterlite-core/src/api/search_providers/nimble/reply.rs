use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::records::{SearchObject, SearchResponse, SearchResult};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NimbleReplyError {
    #[error("the Nimble search reply does not match the /v2/search schema: {source}")]
    Schema {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Deserialize)]
struct NimbleReply {
    results: Vec<NimbleResult>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

// every field is optional so one degraded result empties its fields instead of failing the call
#[derive(Debug, Deserialize)]
struct NimbleResult {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    additional_data: Option<JsonValue>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

pub fn decode_nimble_reply(bytes: &[u8]) -> Result<SearchResponse, NimbleReplyError> {
    let reply: NimbleReply =
        serde_json::from_slice(bytes).map_err(|source| NimbleReplyError::Schema { source })?;
    Ok(SearchResponse {
        results: reply.results.into_iter().map(project).collect(),
        object: SearchObject::default(),
        extra: Object::new(),
        hidden_params: Object::new(),
    })
}

fn project(result: NimbleResult) -> SearchResult {
    let snippet = [result.content, result.description]
        .into_iter()
        .flatten()
        .find(|text| !text.is_empty())
        .unwrap_or_default();
    // only the absolute publish_date is a date; publish_date_raw ("1 day ago") stays in the extra
    let date = result
        .additional_data
        .as_ref()
        .and_then(|data| data.get("publish_date"))
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let mut extra = Object::new();
    if let Some(additional_data) = result.additional_data {
        extra.insert("additional_data".to_owned(), additional_data);
    }
    SearchResult {
        title: result.title.unwrap_or_default(),
        url: result.url.unwrap_or_default(),
        snippet,
        date,
        last_updated: None,
        extra,
    }
}
