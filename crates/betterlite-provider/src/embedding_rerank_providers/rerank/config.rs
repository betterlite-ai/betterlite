use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::cohere_rerank::{self, RerankVersion};
use betterlite_openai::host_rerank::{self, HostProvider};
use betterlite_openai::openai_base_config::Headers;

use super::FamilyRerankConfig;
use super::voyage::{voyage_headers, voyage_params};
use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, MapCohereRerankParams, Params, ResponseTransform,
};
use crate::embedding_rerank_providers::error::FamilyError;

// LiteLLM's supported_rerank_params names, in the order each family's config lists them
const V1_PARAMS: &str = "query documents top_n max_chunks_per_doc rank_fields return_documents";
const V2_PARAMS: &str = "query documents top_n max_tokens_per_doc rank_fields return_documents";
const JINA_PARAMS: &str = "query top_n documents return_documents";
const VOYAGE_PARAMS: &str = "query documents top_n return_documents";

impl BaseRerankConfig for FamilyRerankConfig<'_> {
    type Error = FamilyError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        let headers = match self.config {
            RerankConfig::Cohere | RerankConfig::CohereV2 => {
                cohere_rerank::request_headers(request.headers, request.api_key, self.cohere)?
            }
            RerankConfig::Infinity => {
                host_rerank::infinity_request_headers(request.headers, request.api_key, self.host)
            }
            RerankConfig::JinaAi => host_rerank::jina_request_headers(request.api_key)?,
            RerankConfig::Voyage => voyage_headers(self.environment, request.api_key)?,
        };
        Ok(headers)
    }

    fn transform_rerank_request(
        &self,
        model: &str,
        optional_rerank_params: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        let body = match self.config {
            RerankConfig::Cohere => cohere_rerank::transform_rerank_request(
                RerankVersion::V1,
                model,
                optional_rerank_params,
            )?,
            RerankConfig::CohereV2 => cohere_rerank::transform_rerank_request(
                RerankVersion::V2,
                model,
                optional_rerank_params,
            )?,
            RerankConfig::Infinity => host_rerank::transform_request(
                HostProvider::Infinity,
                model,
                optional_rerank_params,
            )?,
            RerankConfig::JinaAi => {
                host_rerank::transform_request(HostProvider::Jina, model, optional_rerank_params)?
            }
            // LiteLLM's Voyage transform adds the model to the mapped parameters and nothing else
            RerankConfig::Voyage => {
                let mut body = optional_rerank_params.clone();
                body.insert(String::from("model"), Json::Text(model.to_owned()));
                body
            }
        };
        Ok(body)
    }

    fn transform_rerank_response(
        &self,
        _model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        super::decode(
            self.config,
            request.raw_response.status_code,
            &request.raw_response.body,
        )
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        self.url(api_base).unwrap_or_default()
    }

    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        let names = match self.config {
            RerankConfig::Cohere | RerankConfig::Infinity => V1_PARAMS,
            RerankConfig::CohereV2 => V2_PARAMS,
            RerankConfig::JinaAi => JINA_PARAMS,
            RerankConfig::Voyage => VOYAGE_PARAMS,
        };
        names.split_whitespace().map(str::to_owned).collect()
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        let options: &Params = request.non_default_params;
        let params = match self.config {
            RerankConfig::Cohere => {
                cohere_rerank::map_rerank_params(RerankVersion::V1, options, options)
            }
            RerankConfig::CohereV2 => {
                cohere_rerank::map_rerank_params(RerankVersion::V2, options, options)
            }
            RerankConfig::Infinity => {
                host_rerank::map_params(HostProvider::Infinity, options, options)
            }
            RerankConfig::JinaAi => host_rerank::map_params(HostProvider::Jina, options, options),
            RerankConfig::Voyage => voyage_params(&request),
        };
        Ok(params)
    }
}
