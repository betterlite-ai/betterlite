use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreSearchResponse,
};
use betterlite_provider::api::vector_store_contracts::config::{
    AuthCredentials, EndpointsByType, Params, PreparedRequest, SearchQuery, SearchRequest,
    VectorStoreConfig, VectorStoreLogging, VectorStoreResponse,
};
use betterlite_provider::api::vector_store_contracts::error::VectorStoreContractError;
use serde_json::Number;
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error(transparent)]
    Contract(#[from] VectorStoreContractError),
}

pub(super) struct MinimalConfig;

pub(super) struct RecordingConfig;

impl VectorStoreConfig for MinimalConfig {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        let calls = match request.logging.model_call_details.get("calls") {
            Some(Json::Number(number)) => number.as_u64().unwrap_or_default(),
            _ => 0,
        };
        request.logging.model_call_details.insert(
            "calls".to_owned(),
            Json::Number(Number::from(calls.saturating_add(1))),
        );
        let query = match request.query {
            SearchQuery::Text(text) => Json::Text(text.clone()),
            SearchQuery::Sequence(texts) => {
                Json::List(texts.iter().map(|text| Json::Text(text.clone())).collect())
            }
        };
        let body = Params::from([
            (
                "vector_store_id".to_owned(),
                Json::Text(request.vector_store_id.to_owned()),
            ),
            ("query".to_owned(), query),
            (
                "optional_params".to_owned(),
                object(request.optional_params),
            ),
            (
                "api_base".to_owned(),
                Json::Text(request.api_base.to_owned()),
            ),
            ("litellm_params".to_owned(), object(request.litellm_params)),
            (
                "extra_body".to_owned(),
                request.extra_body.map_or(Json::Null, object),
            ),
        ]);
        Ok((format!("{}/vector_stores/search", request.api_base), body))
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }
}

impl VectorStoreConfig for RecordingConfig {
    type Error = FixtureError;

    fn auth_credentials(&self, _litellm_params: &Params) -> AuthCredentials {
        AuthCredentials::default()
    }

    fn endpoints_by_type(&self) -> EndpointsByType {
        EndpointsByType::default()
    }

    fn map_params(
        &self,
        non_default_params: &Params,
        mut optional_params: Params,
        drop_params: bool,
    ) -> Params {
        let observed = Params::from([
            ("non_default_params".to_owned(), object(non_default_params)),
            ("drop_params".to_owned(), Json::Bool(drop_params)),
        ]);
        optional_params.insert(
            "observed".to_owned(),
            Json::Object(observed.into_iter().collect()),
        );
        optional_params
    }

    fn transform_search_request(
        &self,
        _request: SearchRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_search_response(
        &self,
        _response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_request(
        &self,
        _optional_params: &Params,
        _api_base: &str,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }

    fn transform_create_response(
        &self,
        _response: &VectorStoreResponse,
    ) -> Result<VectorStoreCreateResponse, Self::Error> {
        Err(FixtureError::Contract(
            VectorStoreContractError::ApiBaseRequired,
        ))
    }
}

fn object(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}
