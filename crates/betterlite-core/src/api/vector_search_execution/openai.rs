use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::Headers;
use betterlite_openai::vector_stores::complete_url as openai_complete_url;
use betterlite_openai::vector_stores::store::{Query, SearchParams, search_request};
use betterlite_openai::vector_stores::validate_environment as openai_validate_environment;
use secrecy::SecretString;

use super::config::{
    Params, PreparedSearch, ResponseHeaders, SearchConfig, SearchQuery, SearchRequest,
    VectorStoreLogging, VectorStoreResponse,
};
use super::error::{BaseLlmException, HeaderMap, SearchError};
use super::params::{api_key, boolean, integer, present, text};

// OpenAIVectorStoreConfig's search members over B527, which owns the credential and body shapes
pub struct OpenAiSearchConfig {
    settings: OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    litellm_api_base: Option<String>,
}

impl OpenAiSearchConfig {
    #[must_use]
    pub const fn new(
        settings: OpenAiCredentialsSettings,
        globals: LiteLlmGlobals,
        env_api_key: Option<SecretString>,
        litellm_api_base: Option<String>,
    ) -> Self {
        Self {
            settings,
            globals,
            env_api_key,
            litellm_api_base,
        }
    }
}

impl SearchConfig for OpenAiSearchConfig {
    fn validate_environment(
        &self,
        headers: Headers,
        litellm_params: &Params,
    ) -> Result<Headers, SearchError> {
        let explicit = api_key(litellm_params)?;
        Ok(openai_validate_environment(
            headers,
            explicit,
            globals(&self.globals),
            self.env_api_key.clone(),
        ))
    }

    fn complete_url(&self, litellm_params: &Params) -> Result<String, SearchError> {
        let api_base = text(litellm_params, "api_base")?;
        Ok(openai_complete_url(
            api_base,
            self.litellm_api_base.as_deref(),
            &self.settings,
            None,
        )?)
    }

    fn transform_search_request(
        &self,
        request: SearchRequest<'_>,
    ) -> Result<PreparedSearch, SearchError> {
        let query = match request.query {
            SearchQuery::Text(value) => Query::One(value.clone()),
            SearchQuery::Sequence(parts) => Query::Many(parts.clone()),
        };
        let params = search_params(request.optional_params)?;
        let prepared = search_request(request.vector_store_id, &query, &params, request.api_base)?;
        let body = serde_json::from_value(prepared.body).map_err(SearchError::Encode)?;
        Ok((prepared.url, body))
    }

    fn transform_search_response(
        &self,
        response: &VectorStoreResponse,
        _logging: &mut VectorStoreLogging,
    ) -> Result<VectorStoreSearchResponse, SearchError> {
        serde_json::from_slice(&response.body).map_err(|source| {
            SearchError::Provider(self.error_class(
                &source.to_string(),
                response.status_code,
                response_headers(&response.headers),
            ))
        })
    }

    // BaseVectorStoreConfig.get_error_class raises this exception, so asking it and building one are the same answer here
    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

// B527 owns the credential source rules, so the globals are copied into the shape its resolver reads
fn globals(source: &LiteLlmGlobals) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: source.api_base.clone(),
        api_key: source.api_key.clone(),
        openai_key: source.openai_key.clone(),
        organization: source.organization.clone(),
    }
}

// VectorStoreSearchRequest reads each declared key with .get(key, None), so an explicit null is the absent value
fn search_params(params: &Params) -> Result<SearchParams, SearchError> {
    let mut typed = SearchParams::default();
    for (key, value) in params {
        match key.as_str() {
            "filters" => {
                if let Some(given) = present(value) {
                    typed.filters = Some(serde_json::to_value(given).map_err(SearchError::Encode)?);
                }
            }
            "max_num_results" => typed.max_num_results = integer(value, "max_num_results")?,
            "ranking_options" => {
                if let Some(given) = present(value) {
                    typed.ranking_options =
                        Some(serde_json::to_value(given).map_err(SearchError::Encode)?);
                }
            }
            "rewrite_query" => typed.rewrite_query = boolean(value, "rewrite_query")?,
            _ => {}
        }
    }
    Ok(typed)
}

fn response_headers(headers: &ResponseHeaders) -> HeaderMap {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
