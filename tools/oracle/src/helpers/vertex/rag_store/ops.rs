use std::collections::BTreeMap;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_models::decisions::Json;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_rag_store::records::create::transform_create_request;
use betterlite_vertex::vertex_rag_store::records::payload::{
    Reply, transform_create_response, transform_search_response,
};
use betterlite_vertex::vertex_rag_store::records::search::{
    SearchQuery, SearchRequest, transform_search_request,
};
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Search {
        vector_store_id: String,
        query: SearchQuery,
        api_base: String,
        #[serde(default)]
        litellm_params: BTreeMap<String, Json>,
        #[serde(default)]
        optional_params: BTreeMap<String, Json>,
        #[serde(default)]
        response: Option<ReplyCall>,
    },
    CreateRequest {
        api_base: String,
        #[serde(default)]
        optional_params: BTreeMap<String, Json>,
    },
    CreateResponse(ReplyCall),
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

fn outcome(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Search {
            vector_store_id,
            query,
            api_base,
            litellm_params,
            optional_params,
            response,
        } => {
            let mut logging = BTreeMap::new();
            let request = transform_search_request(SearchRequest {
                vector_store_id,
                query,
                optional_params,
                api_base,
                logging: &mut logging,
                params: vertex_params(litellm_params),
                settings: &VertexSettings::default(),
            })
            .or_value_error()?;
            let body = serde_json::to_value(&request.body).or_value_error()?;
            let Some(response) = response else {
                return Ok(json!({"url": request.url, "body": body}));
            };
            let reply = Reply {
                status: response.status,
                headers: &response.headers,
                body: response.body.as_bytes(),
            };
            let results = transform_search_response(&reply, &logging).or_value_error()?;
            let results = serde_json::to_value(&results).or_value_error()?;
            Ok(json!({"url": request.url, "body": body, "results": results}))
        }
        Call::CreateRequest {
            api_base,
            optional_params,
        } => {
            let call = transform_create_request(optional_params, api_base).or_value_error()?;
            Ok(json!({
                "url": call.url,
                "body": serde_json::to_value(&call.body).or_value_error()?,
            }))
        }
        Call::CreateResponse(raw) => {
            let reply = Reply {
                status: raw.status,
                headers: &raw.headers,
                body: raw.body.as_bytes(),
            };
            let local = LocalTimeContext {
                zone: TimeZone::UTC,
            };
            let created = transform_create_response(&reply, &local).or_value_error()?;
            Ok(json!({"created": serde_json::to_value(&created).or_value_error()?}))
        }
    }
}

fn vertex_params(fields: &BTreeMap<String, Json>) -> VertexParams<'_> {
    VertexParams {
        project: text(fields, "vertex_project"),
        ai_project: text(fields, "vertex_ai_project"),
        location: text(fields, "vertex_location"),
        ai_location: text(fields, "vertex_ai_location"),
        credentials: None,
        ai_credentials: None,
    }
}

fn text<'fields>(fields: &'fields BTreeMap<String, Json>, name: &str) -> Option<&'fields str> {
    match fields.get(name) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}
