use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankResponse, RerankResponseDocument, RerankResponseMeta,
    RerankResponseResult, RerankTokens,
};
use betterlite_openai::api::vector_store_records::records::score::Score;
use betterlite_openai::dashscope_embedding::error::DashscopeError;
use betterlite_openai::dashscope_embedding::identity::{
    FamilyIdentity, FamilyKeyError, resolve_key,
};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;
use thiserror::Error;
use uuid::Uuid;

use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params, ResponseTransform,
};
use crate::dashscope_common::resolve_rerank_base;

pub const SUPPORTED_RERANK_PARAMS: [&str; 4] = ["query", "documents", "top_n", "return_documents"];
const AUTHORIZATION_HEADER: &str = "Authorization";
const ACCEPT_HEADER: &str = "accept";
const CONTENT_TYPE_HEADER: &str = "content-type";
const JSON_CONTENT_TYPE: &str = "application/json";
// a reply the rerank records cannot hold is an unclassified provider failure, which LiteLLM answers with 500
const SHAPE_FAILURE_STATUS: u16 = 500;

mod tests;

#[derive(Debug, Error)]
pub enum DashscopeRerankError {
    #[error(transparent)]
    Provider(#[from] DashscopeError),
    #[error(transparent)]
    Key(#[from] FamilyKeyError),
    #[error("query is required for DashScope rerank")]
    MissingQuery,
    #[error("documents is required for DashScope rerank")]
    MissingDocuments,
}

pub struct DashscopeRerankConfig {
    pub identity: FamilyIdentity,
    pub rerank_env_value: String,
    pub default_rerank_url: String,
}

impl BaseRerankConfig for DashscopeRerankConfig {
    type Error = DashscopeRerankError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        let key = resolve_key(
            &self.identity,
            request.api_key.map(|key| key.expose_secret().as_str()),
        )?;
        let mut headers = Headers::new();
        headers.insert(AUTHORIZATION_HEADER.to_owned(), format!("Bearer {key}"));
        headers.insert(ACCEPT_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        headers.extend(
            request
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        Ok(headers)
    }

    fn transform_rerank_request(
        &self,
        model: &str,
        optional_rerank_params: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        let query = optional_rerank_params
            .get("query")
            .cloned()
            .ok_or(DashscopeRerankError::MissingQuery)?;
        let documents = optional_rerank_params
            .get("documents")
            .cloned()
            .ok_or(DashscopeRerankError::MissingDocuments)?;
        let mut body = Params::new();
        body.insert("model".to_owned(), Json::Text(model.to_owned()));
        body.insert("query".to_owned(), query);
        body.insert("documents".to_owned(), documents);
        for name in ["top_n", "return_documents"] {
            let Some(value) = optional_rerank_params.get(name) else {
                continue;
            };
            if *value == Json::Null {
                continue;
            }
            body.insert(name.to_owned(), value.clone());
        }
        Ok(body)
    }

    fn transform_rerank_response(
        &self,
        _model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        let raw = request.raw_response;
        decode_dashscope_rerank_reply(&raw.body, raw.status_code)
            .map_err(DashscopeRerankError::Provider)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        let resolved =
            resolve_rerank_base(api_base, &self.rerank_env_value, &self.default_rerank_url);
        if resolved == self.default_rerank_url {
            return resolved;
        }
        let cleaned = resolved.trim_end_matches('/');
        if cleaned.ends_with("/reranks") || cleaned.ends_with("/rerank") {
            return cleaned.to_owned();
        }
        format!("{cleaned}/reranks")
    }

    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        SUPPORTED_RERANK_PARAMS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        let mut params = Params::new();
        params.insert("query".to_owned(), Json::Text(request.query.to_owned()));
        params.insert(
            "documents".to_owned(),
            Json::List(
                request
                    .documents
                    .iter()
                    .map(|document| document.as_json())
                    .collect(),
            ),
        );
        if let Some(top_n) = request.top_n {
            params.insert("top_n".to_owned(), Json::Number(Number::from(top_n)));
        }
        if let Some(return_documents) = request.return_documents {
            params.insert("return_documents".to_owned(), Json::Bool(return_documents));
        }
        Ok(params)
    }
}

pub fn decode_dashscope_rerank_reply(
    bytes: &[u8],
    status_code: u16,
) -> Result<RerankResponse, DashscopeError> {
    let body: Json = serde_json::from_slice(bytes).map_err(|_| DashscopeError {
        status_code,
        message: body_text(bytes),
        headers: Headers::new(),
    })?;
    let Json::Object(fields) = &body else {
        return Err(shape_error("the reply"));
    };
    let results = fields.get("results");
    if fields.contains_key("code") && results.is_none() {
        return Err(DashscopeError {
            status_code,
            message: code_message(fields, &body),
            headers: Headers::new(),
        });
    }
    let items = match results {
        None | Some(Json::Null) => return Err(no_results_error(status_code, &body)),
        Some(Json::List(items)) => items,
        Some(_) => return Err(shape_error("results")),
    };
    let mut transformed = Vec::with_capacity(items.len());
    for item in items {
        transformed.push(rerank_result(item)?);
    }
    let total_tokens = total_tokens(fields.get("usage"))?;
    let id = match fields.get("id") {
        Some(Json::Text(text)) if !text.is_empty() => text.clone(),
        _ => Uuid::new_v4().to_string(),
    };
    let meta = match total_tokens {
        Some(total) => RerankResponseMeta {
            api_version: None,
            billed_units: Some(Some(RerankBilledUnits {
                search_units: None,
                total_tokens: Some(Some(total)),
            })),
            tokens: Some(Some(RerankTokens {
                input_tokens: Some(Some(total)),
                output_tokens: None,
            })),
        },
        None => RerankResponseMeta::default(),
    };
    Ok(RerankResponse {
        id: Some(id),
        results: Some(transformed),
        meta: Some(meta),
        hidden_params: BTreeMap::new(),
    })
}

fn rerank_result(item: &Json) -> Result<RerankResponseResult, DashscopeError> {
    let Json::Object(fields) = item else {
        return Err(shape_error("results"));
    };
    let index = match fields.get("index") {
        Some(Json::Number(number)) => number.as_i64(),
        _ => None,
    };
    let Some(index) = index else {
        return Err(shape_error("results[].index"));
    };
    let relevance_score = match fields.get("relevance_score") {
        Some(Json::Number(number)) => Score::try_from(number.clone()).ok(),
        _ => None,
    };
    let Some(relevance_score) = relevance_score else {
        return Err(shape_error("results[].relevance_score"));
    };
    let document = match fields.get("document") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(RerankResponseDocument { text: text.clone() }),
        Some(Json::Object(document)) => match document.get("text") {
            Some(Json::Text(text)) => Some(RerankResponseDocument { text: text.clone() }),
            _ => return Err(shape_error("results[].document.text")),
        },
        Some(_) => None,
    };
    Ok(RerankResponseResult {
        index,
        relevance_score,
        document,
    })
}

fn total_tokens(usage: Option<&Json>) -> Result<Option<i64>, DashscopeError> {
    match usage {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => match fields.get("total_tokens") {
            None | Some(Json::Null) => Ok(None),
            Some(Json::Number(number)) => number
                .as_i64()
                .map(Some)
                .ok_or_else(|| shape_error("usage.total_tokens")),
            Some(_) => Err(shape_error("usage.total_tokens")),
        },
        Some(_) => Err(shape_error("usage")),
    }
}

fn code_message(fields: &IndexMap<String, Json>, body: &Json) -> String {
    match fields.get("message") {
        Some(Json::Text(message)) => message.clone(),
        _ => serde_json::to_string(body).unwrap_or_default(),
    }
}

fn no_results_error(status_code: u16, body: &Json) -> DashscopeError {
    DashscopeError {
        status_code,
        message: format!(
            "No results in DashScope rerank response: {}",
            serde_json::to_string(body).unwrap_or_default()
        ),
        headers: Headers::new(),
    }
}

fn body_text(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).map_or_else(
        |_| "the DashScope rerank reply is not UTF-8 text".to_owned(),
        str::to_owned,
    )
}

fn shape_error(field: &str) -> DashscopeError {
    DashscopeError {
        status_code: SHAPE_FAILURE_STATUS,
        message: format!("the DashScope rerank reply field `{field}` has the wrong type"),
        headers: Headers::new(),
    }
}
