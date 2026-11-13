use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::dashscope_embedding::error::DashscopeError;
use betterlite_openai::dashscope_embedding::identity::{
    FamilyIdentity, FamilyKeyError, resolve_key,
};
use secrecy::ExposeSecret as _;
use serde_json::Number;
use thiserror::Error;

use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params, ResponseTransform,
};
use crate::dashscope_common::resolve_rerank_base;

pub mod decode;

pub const SUPPORTED_RERANK_PARAMS: [&str; 4] = ["query", "documents", "top_n", "return_documents"];
const AUTHORIZATION_HEADER: &str = "Authorization";
const ACCEPT_HEADER: &str = "accept";
const CONTENT_TYPE_HEADER: &str = "content-type";
const JSON_CONTENT_TYPE: &str = "application/json";

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
        decode::decode_dashscope_rerank_reply(&raw.body, raw.status_code)
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
