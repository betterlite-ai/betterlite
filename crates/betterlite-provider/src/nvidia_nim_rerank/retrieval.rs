use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use super::reply::{decode_nvidia_nim_rerank_reply, to_response};
use super::request::{cohere_rerank_params, rerank_body};
use super::{
    DEFAULT_NIM_RERANK_API_BASE, NVIDIA_NIM_PREFIX, NvidiaNimRerankError, SUPPORTED_COHERE_PARAMS,
    TEXT_PASSAGE_FIELDS,
};
use crate::api::rerank_contracts::{
    BaseLlmException, BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params,
    ResponseTransform,
};

const AUTHORIZATION: &str = "Authorization";
const ACCEPT: &str = "accept";
const CONTENT_TYPE: &str = "content-type";
const JSON_CONTENT_TYPE: &str = "application/json";
const RETRIEVAL_PATH: &str = "/retrieval/";
const V1_SUFFIX: &str = "/v1";

// LiteLLM's NvidiaNimRerankConfig reads the environment and litellm.api_key; here the caller passes both
#[derive(Debug, Clone)]
pub struct NvidiaNimRerankConfig {
    pub env_api_key: Option<SecretString>,
    pub global_api_key: Option<SecretString>,
    pub new_id: fn() -> Uuid,
}

impl Default for NvidiaNimRerankConfig {
    fn default() -> Self {
        Self {
            env_api_key: None,
            global_api_key: None,
            new_id: Uuid::new_v4,
        }
    }
}

impl NvidiaNimRerankConfig {
    pub fn validate_environment_with_keys(
        headers: Headers,
        api_key: Option<&SecretString>,
        env_api_key: Option<&SecretString>,
        global_api_key: Option<&SecretString>,
    ) -> Result<Headers, NvidiaNimRerankError> {
        let Some(api_key) = api_key.or(env_api_key).or(global_api_key) else {
            return Err(NvidiaNimRerankError::MissingApiKey);
        };
        let mut merged = Headers::from([
            (
                AUTHORIZATION.to_owned(),
                format!("Bearer {}", api_key.expose_secret()),
            ),
            (ACCEPT.to_owned(), JSON_CONTENT_TYPE.to_owned()),
            (CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned()),
        ]);
        merged.extend(headers);
        Ok(merged)
    }
}

impl BaseRerankConfig for NvidiaNimRerankConfig {
    type Error = NvidiaNimRerankError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        Self::validate_environment_with_keys(
            request.headers.clone(),
            request.api_key,
            self.env_api_key.as_ref(),
            self.global_api_key.as_ref(),
        )
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        let configured = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(DEFAULT_NIM_RERANK_API_BASE);
        let trimmed = configured.trim_end_matches('/');
        if trimmed.contains(RETRIEVAL_PATH) {
            return trimmed.to_owned();
        }
        let root = trimmed.strip_suffix(V1_SUFFIX).unwrap_or(trimmed);
        format!(
            "{root}/v1/retrieval/{}/reranking",
            model.strip_prefix(NVIDIA_NIM_PREFIX).unwrap_or(model)
        )
    }

    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        SUPPORTED_COHERE_PARAMS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        Ok(cohere_rerank_params(
            request.query,
            request.documents,
            request.non_default_params,
            request.top_n,
        ))
    }

    fn transform_rerank_request(
        &self,
        model: &str,
        optional_rerank_params: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        let clean_model = model.strip_prefix(NVIDIA_NIM_PREFIX).unwrap_or(model);
        rerank_body(clean_model, optional_rerank_params, &TEXT_PASSAGE_FIELDS)
    }

    fn transform_rerank_response(
        &self,
        _model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        let reply = match decode_nvidia_nim_rerank_reply(&request.raw_response.body) {
            Ok(reply) => reply,
            Err(NvidiaNimRerankError::ReplyNotJson { .. }) => {
                return Err(BaseLlmException {
                    status_code: request.raw_response.status_code,
                    message: std::str::from_utf8(&request.raw_response.body).map_or_else(
                        |_| "the rerank reply body is not valid UTF-8".to_owned(),
                        str::to_owned,
                    ),
                    headers: request
                        .raw_response
                        .headers
                        .iter()
                        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                        .collect(),
                }
                .into());
            }
            Err(error) => return Err(error),
        };
        to_response(&reply, request.request_data, self.new_id)
    }
}
