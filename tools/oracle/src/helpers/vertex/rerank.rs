use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::probe::AccessToken;
use betterlite_vertex::vertex_rerank::request::{map_rerank_params, transform_request};
use betterlite_vertex::vertex_rerank::response::{decode_vertex_rerank, transform_response};
use betterlite_vertex::vertex_rerank::target::{complete_url, request_headers};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    #[serde(rename = "api_base")]
    _api_base: Option<String>,
    vertex_project: Option<String>,
    vertex_credentials: Option<String>,
    access_project: String,
    headers: BTreeMap<String, String>,
    query: String,
    documents: Vec<RerankDocument>,
    top_n: Option<i64>,
    return_documents: Option<bool>,
    non_default_params: Extra,
    litellm_params: Option<Json>,
    response: Json,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime
            .block_on(async { Ok(json!({"target": target(input).await?, "rank": rank(input)?})) })
    }
}

// the access token lookup is scripted on both sides: a fixed token, the project the case names, and a record of what it was asked
fn scripted<'run>(
    lookups: &'run Mutex<Vec<Value>>,
    project: &'run str,
) -> impl FnOnce(
    Option<&ConfiguredCredentials>,
    Option<&str>,
) -> Ready<Result<AccessToken, CredentialCacheError>>
+ 'run {
    move |credentials: Option<&ConfiguredCredentials>, requested: Option<&str>| {
        lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "credentials": credentials.and_then(ConfiguredCredentials::as_text),
                "project": requested,
                "provider": "vertex_ai",
            }));
        ready(Ok(AccessToken {
            token: SecretString::from("token"),
            project: project.to_owned(),
        }))
    }
}

async fn target(input: &Input) -> HelperResult {
    let credentials = input
        .vertex_credentials
        .as_deref()
        .map(|text| ConfiguredCredentials::Text(SecretString::from(text)));
    let params = VertexParams {
        project: input.vertex_project.as_deref(),
        credentials: credentials.as_ref(),
        ..VertexParams::default()
    };
    let vertex = VertexSettings::default();
    let lookups = Mutex::new(Vec::new());
    let url = complete_url(params, &vertex, scripted(&lookups, &input.access_project))
        .await
        .or_value_error()?;
    let headers = request_headers(
        params,
        &vertex,
        &input.headers,
        scripted(&lookups, &input.access_project),
    )
    .await
    .or_value_error()?;
    let exposed: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect();
    Ok(json!({
        "lookups": lookups.into_inner().unwrap_or_else(PoisonError::into_inner),
        "url": url,
        "headers": exposed,
    }))
}

fn rank(input: &Input) -> HelperResult {
    let mapped = map_rerank_params(
        &input.non_default_params,
        &input.query,
        &input.documents,
        input.top_n,
        input.return_documents,
    );
    let request =
        transform_request(&input.model, &mapped, input.litellm_params.as_ref()).or_value_error()?;
    let bytes = serde_json::to_vec(&input.response).or_value_error()?;
    let records = decode_vertex_rerank(&bytes).or_value_error()?;
    let answer = transform_response(records, &request, Uuid::from_u128(1));
    // the oracle compares scores as normalized decimal text, since JSON 1 and 1.0 differ
    let results: Vec<Value> = answer
        .results
        .unwrap_or_default()
        .iter()
        .map(|entry| {
            json!({
                "index": entry.index,
                "relevance_score": entry.relevance_score.0.normalize().to_string(),
            })
        })
        .collect();
    let search_units = answer
        .meta
        .and_then(|meta| meta.billed_units)
        .flatten()
        .and_then(|billed| billed.search_units)
        .flatten();
    Ok(json!({
        "request": serde_json::to_value(&request).or_value_error()?,
        "response": {"id": answer.id, "results": results, "search_units": search_units},
    }))
}
