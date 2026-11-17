use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::openai_base_config::{Headers, Object as BaseObject};
use betterlite_openai::voyage_rerank;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Number;
use uuid::Uuid;

use crate::api::rerank_contracts::{MapCohereRerankParams, Params};
use crate::embedding_rerank_providers::error::FamilyError;
use crate::embedding_rerank_providers::request::text_headers;

// Voyage answers with data, so LiteLLM rewrites every result and its usage into the shared shape
pub(super) fn voyage_response(status: u16, body: &[u8]) -> Result<RerankResponse, FamilyError> {
    let mut logging = VoyageLogging;
    let answer = voyage_rerank::decode_voyage_rerank(body, status, Uuid::new_v4(), &mut logging)?;
    let mut results = Vec::with_capacity(answer.results.len());
    for result in answer.results {
        let document = match result.document {
            None | Some(Json::Null) => None,
            // the leaf turns a string document into the text object, and any other object would lose keys
            Some(Json::Object(fields)) => match fields.get("text") {
                Some(Json::Text(text)) if fields.len() == 1 => {
                    Some(RerankResponseDocument { text: text.clone() })
                }
                Some(_) | None => return Err(FamilyError::VoyageDocument),
            },
            Some(Json::Text(text)) => Some(RerankResponseDocument { text }),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
                return Err(FamilyError::VoyageDocument);
            }
        };
        results.push(RerankResponseResult {
            index: result.index,
            relevance_score: Score(result.relevance_score),
            document,
        });
    }
    Ok(RerankResponse {
        id: Some(answer.id),
        results: Some(results),
        meta: Some(RerankResponseMeta {
            api_version: None,
            billed_units: Some(Some(RerankBilledUnits {
                search_units: None,
                total_tokens: Some(answer.meta.billed_units.total_tokens),
            })),
            tokens: Some(Some(RerankTokens {
                input_tokens: Some(answer.meta.tokens.input_tokens),
                output_tokens: Some(Some(answer.meta.tokens.output_tokens)),
            })),
        }),
        hidden_params: BTreeMap::new(),
    })
}

struct VoyageLogging;

impl voyage_rerank::RerankLogging for VoyageLogging {
    fn post_call(&mut self, _original_response: &[u8]) {}
}

// Voyage's own map: query and documents always, top_k and return_documents when the caller set them
pub(super) fn voyage_params(request: &MapCohereRerankParams<'_>) -> Params {
    let documents = request
        .documents
        .iter()
        .map(RerankDocument::as_json)
        .collect();
    let mut params = Params::new();
    params.insert(String::from("query"), Json::Text(request.query.to_owned()));
    params.insert(String::from("documents"), Json::List(documents));
    if let Some(top_n) = request.top_n {
        params.insert(String::from("top_k"), Json::Number(Number::from(top_n)));
    }
    if let Some(return_documents) = request.return_documents {
        params.insert(
            String::from("return_documents"),
            Json::Bool(return_documents),
        );
    }
    params
}

pub(super) fn voyage_headers(
    environment: &(dyn EnvironmentLookup + Send + Sync),
    api_key: Option<&SecretString>,
) -> Result<Headers, voyage_rerank::VoyageRerankError> {
    let key = voyage_rerank::api_key(api_key.map(ExposeSecret::expose_secret), |name| {
        environment.lookup(name)
    })?;
    Ok(text_headers(voyage_rerank::headers(
        BaseObject::new(),
        &key,
    )))
}
