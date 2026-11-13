use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use serde_json::Number;

use super::request::rerank_body;
use super::retrieval::NvidiaNimRerankConfig;
use super::{
    DEFAULT_NIM_RERANK_API_BASE, NVIDIA_NIM_PREFIX, NvidiaNimRerankError, RANKING_PASSAGE_FIELDS,
    RANKING_PREFIX,
};
use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params, ResponseTransform,
};

const TOP_N: &str = "top_n";
const TOP_K: &str = "top_k";
const RANKING_SUFFIX: &str = "/ranking";
const V1_SUFFIX: &str = "/v1";

// LiteLLM keeps top_n on the per-request config instance and strips it from the /v1/ranking body
#[derive(Debug)]
pub struct NvidiaNimRankingConfig {
    pub retrieval: NvidiaNimRerankConfig,
    pub top_n: Mutex<Option<i64>>,
}

impl Default for NvidiaNimRankingConfig {
    fn default() -> Self {
        Self {
            retrieval: NvidiaNimRerankConfig::default(),
            top_n: Mutex::new(None),
        }
    }
}

impl BaseRerankConfig for NvidiaNimRankingConfig {
    type Error = NvidiaNimRerankError;

    fn validate_environment(&self, request: EnvironmentRequest<'_>) -> Result<Headers, Self::Error> {
        self.retrieval.validate_environment(request)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        let configured = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(DEFAULT_NIM_RERANK_API_BASE);
        let trimmed = configured.trim_end_matches('/');
        if trimmed.ends_with(RANKING_SUFFIX) {
            return trimmed.to_owned();
        }
        let root = trimmed.strip_suffix(V1_SUFFIX).unwrap_or(trimmed);
        format!("{root}/v1{RANKING_SUFFIX}")
    }

    fn supported_rerank_params(&self, model: &str) -> Vec<String> {
        self.retrieval.supported_rerank_params(model)
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        let mut params = Params::from([
            ("query".to_owned(), Json::Text(request.query.to_owned())),
            (
                "documents".to_owned(),
                Json::List(
                    request
                        .documents
                        .iter()
                        .map(RerankDocument::as_json)
                        .collect(),
                ),
            ),
        ]);
        params.extend(
            request
                .non_default_params
                .iter()
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        params.remove(TOP_K);
        if let Some(top_n) = request.top_n {
            params.insert(TOP_N.to_owned(), Json::Number(Number::from(top_n)));
        }
        Ok(params)
    }

    fn transform_rerank_request(
        &self,
        model: &str,
        optional_rerank_params: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        match optional_rerank_params.get(TOP_N) {
            None | Some(Json::Null) => {}
            Some(value) => {
                let Json::Number(number) = value else {
                    return Err(NvidiaNimRerankError::TopNNotPositive {
                        value: serde_json::to_string(value).unwrap_or_default(),
                    });
                };
                let top_n = number.as_i64().filter(|top_n| *top_n >= 1).ok_or_else(|| {
                    NvidiaNimRerankError::TopNNotPositive {
                        value: serde_json::to_string(value).unwrap_or_default(),
                    }
                })?;
                *self.top_n.lock().unwrap_or_else(PoisonError::into_inner) = Some(top_n);
            }
        }
        let without_provider = model.strip_prefix(NVIDIA_NIM_PREFIX).unwrap_or(model);
        let clean_model = without_provider
            .strip_prefix(RANKING_PREFIX)
            .unwrap_or(without_provider);
        let filtered: Params = optional_rerank_params
            .iter()
            .filter(|(name, _)| name.as_str() != TOP_N && name.as_str() != TOP_K)
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        rerank_body(clean_model, &filtered, &RANKING_PASSAGE_FIELDS)
    }

    fn transform_rerank_response(
        &self,
        model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        let from_params = match request.optional_params.get(TOP_N) {
            Some(Json::Number(number)) => number.as_i64().filter(|top_n| *top_n > 0),
            Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
            | None => None,
        };
        let mut response = self.retrieval.transform_rerank_response(model_response, request)?;
        let kept = *self.top_n.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(top_n) = from_params.or(kept) else {
            return Ok(response);
        };
        let Ok(limit) = usize::try_from(top_n) else {
            return Ok(response);
        };
        let Some(results) = response.results.as_mut() else {
            return Ok(response);
        };
        if results.len() > limit {
            results.sort_by(|left, right| right.relevance_score.0.cmp(&left.relevance_score.0));
            results.truncate(limit);
        }
        Ok(response)
    }
}
