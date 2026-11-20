use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::metadata_filter_limit;
use betterlite_openai::vector_stores::files::{FileOperation, file_request};
use betterlite_openai::vector_stores::{
    Headers, complete_url as openai_complete_url,
    validate_environment as openai_validate_environment,
};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, UrlFailure, header_map,
};

use super::error::FileWriteError;

// The query a provider's delete transform answers, in httpx's dict order
pub type QueryParams = IndexMap<String, String>;

// The litellm_params fields the four handlers read
#[derive(Debug, Clone, Default)]
pub struct WriteParams {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub ssl_verify: Option<bool>,
}

// The BaseVectorStoreFilesConfig members these handlers call; a provider config adapts to this
pub trait FileWriteConfig: Send + Sync {
    fn validate_environment(&self, headers: Headers, params: &WriteParams) -> Headers;

    fn complete_url(
        &self,
        params: &WriteParams,
        vector_store_id: &str,
    ) -> Result<String, UrlFailure>;

    fn transform_update_request(
        &self,
        request: &IndexMap<String, Json>,
        api_base: &str,
        file_id: &str,
    ) -> Result<(String, IndexMap<String, Json>), FileWriteError>;

    fn transform_update_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException>;

    fn transform_delete_request(
        &self,
        api_base: &str,
        file_id: &str,
    ) -> Result<(String, QueryParams), FileWriteError>;

    fn transform_delete_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException>;

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException;
}

// OpenAIVectorStoreFilesConfig's members over B527, whose validate_environment rewrites Authorization and Content-Type in the map it is given
pub struct OpenAiFileWriteConfig<'settings> {
    settings: &'settings OpenAiCredentialsSettings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    litellm_api_base: Option<String>,
}

impl<'settings> OpenAiFileWriteConfig<'settings> {
    #[must_use]
    pub const fn new(
        settings: &'settings OpenAiCredentialsSettings,
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

impl FileWriteConfig for OpenAiFileWriteConfig<'_> {
    fn validate_environment(&self, headers: Headers, params: &WriteParams) -> Headers {
        openai_validate_environment(
            headers,
            params.api_key.clone(),
            LiteLlmGlobals {
                api_base: self.globals.api_base.clone(),
                api_key: self.globals.api_key.clone(),
                openai_key: self.globals.openai_key.clone(),
                organization: self.globals.organization.clone(),
            },
            self.env_api_key.clone(),
        )
    }

    fn complete_url(
        &self,
        params: &WriteParams,
        vector_store_id: &str,
    ) -> Result<String, UrlFailure> {
        openai_complete_url(
            params.api_base.as_deref(),
            self.litellm_api_base.as_deref(),
            self.settings,
            Some(vector_store_id),
        )
        .map_err(|source| UrlFailure {
            source: Box::new(source),
        })
    }

    // dict(update_request) keeps its keys and nulls; only a mapping under attributes is cleaned, and extra_body already overlaid it
    fn transform_update_request(
        &self,
        request: &IndexMap<String, Json>,
        api_base: &str,
        file_id: &str,
    ) -> Result<(String, IndexMap<String, Json>), FileWriteError> {
        let mut payload = request.clone();
        if let Some(Json::Object(attributes)) = request.get("attributes") {
            payload.insert(String::from("attributes"), cleaned_attributes(attributes)?);
        }
        let encoded = encode_segment(file_id, "file_id")?;
        Ok((format!("{api_base}/{encoded}"), payload))
    }

    fn transform_update_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        self.reply(response)
    }

    // OpenAI's delete transform answers the collection URL plus one encoded file id and an empty params dict
    fn transform_delete_request(
        &self,
        api_base: &str,
        file_id: &str,
    ) -> Result<(String, QueryParams), FileWriteError> {
        let request = file_request(&FileOperation::Delete, file_id, api_base)?;
        Ok((request.url, QueryParams::new()))
    }

    fn transform_delete_response(
        &self,
        response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        self.reply(response)
    }

    // BaseVectorStoreFilesConfig.get_error_class raises this exception, so asking it and building one are the same answer here
    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

impl OpenAiFileWriteConfig<'_> {
    // response.json() inside the transform's try, so a body that does not parse becomes the provider's error class at the reply's status
    fn reply(&self, response: &http::Response<Vec<u8>>) -> Result<Json, BaseLlmException> {
        serde_json::from_slice(response.body()).map_err(|source| {
            self.error_class(
                &source.to_string(),
                response.status().as_u16(),
                header_map(response.headers()),
            )
        })
    }
}

// add_openai_metadata over the mapping: strings only, no hidden_params, the first 16, and an empty dict stays an empty dict
fn cleaned_attributes(attributes: &IndexMap<String, Json>) -> Result<Json, FileWriteError> {
    let encoded = serde_json::to_value(attributes).map_err(FileWriteError::Encode)?;
    let values = serde_json::from_value(encoded).map_err(FileWriteError::Encode)?;
    let cleaned = metadata_filter_limit(Some(&values));
    let filtered = serde_json::to_value(&cleaned).map_err(FileWriteError::Encode)?;
    serde_json::from_value(filtered).map_err(FileWriteError::Encode)
}
