use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_config::sections::studio_identity::Settings as StudioSettings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_core::api::genai_http::config::{AuthCall, AuthTarget, GenerateContentHttpConfig};
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, header_map, header_text,
};
use betterlite_core::genai_setup::config::{GenerateContentProviderConfig, GenerateContentRequest};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::genai::genai_contract::config::TOP_LEVEL_FIELDS;
use betterlite_vertex::google_native_content::auth::{
    NativeAuth, auth_token_and_url as native_auth,
};
use betterlite_vertex::google_native_content::capabilities::NativeProvider;
use betterlite_vertex::google_native_content::error::NativeContentError;
use betterlite_vertex::google_native_content::map::{
    studio_optional_params, vertex_optional_params,
};
use betterlite_vertex::google_native_content::request::{
    GenerateContentRequest as VertexRequest, transform_request as vertex_transform,
};
use betterlite_vertex::google_native_content::response::{
    GenerateContentResponse, RawResponse, transform_response,
};
use betterlite_vertex::studio_identity::{AuthValue, NativeParams};
use betterlite_vertex::token_cache::probe::AccessToken;
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum ConfigError {
    #[error(transparent)]
    Native(#[from] NativeContentError),
    #[error("the request body cannot be written as JSON: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("the b3488 helper sends Gemini requests only")]
    UnsupportedProvider,
}

// The adapter over betterlite-vertex that the handler calls: B3483's request transform, the native auth and the
// transformation, plus the base config's error class that _handle_error raises
pub(super) struct NativeConfig {
    catalog: Arc<ModelCatalog>,
    provider: NativeProvider,
}

impl NativeConfig {
    pub(super) fn new(catalog: Arc<ModelCatalog>) -> Self {
        Self {
            catalog,
            provider: NativeProvider::Studio,
        }
    }
}

impl GenerateContentProviderConfig for NativeConfig {
    type Error = ConfigError;

    fn map_optional_params(&self, config: &Object, _model: &str) -> Object {
        match self.provider {
            NativeProvider::Studio => studio_optional_params(config),
            NativeProvider::Vertex => vertex_optional_params(config),
        }
    }

    fn transform_request(&self, request: &GenerateContentRequest<'_>) -> Result<Json, Self::Error> {
        let native = VertexRequest {
            model: request.model.to_owned(),
            contents: request.contents.clone(),
            tools: request.tools.cloned().unwrap_or(Json::Null),
            generation_config: request.generation_config.clone(),
            system_instruction: request.system_instruction.cloned().unwrap_or(Json::Null),
        };
        let body = vertex_transform(self.provider, &native, &SchemaSettings::default())?;
        Ok(serde_json::from_value(serde_json::to_value(body)?)?)
    }

    fn request_top_level_fields(&self) -> &'static [&'static str] {
        &TOP_LEVEL_FIELDS
    }
}

impl GenerateContentHttpConfig for NativeConfig {
    type Response = GenerateContentResponse;

    // sync_get_auth_token_and_url: the Studio key names the header, and Gemini's access token is empty
    fn auth_token_and_url(
        &self,
        call: &AuthCall<'_>,
    ) -> impl Future<Output = Result<AuthTarget, ConfigError>> + Send {
        async move {
            if let NativeProvider::Vertex = self.provider {
                return Err(ConfigError::UnsupportedProvider);
            }
            let vertex = VertexSettings::default();
            let studio = StudioSettings::default();
            let auth = NativeAuth {
                provider: self.provider,
                model: call.model,
                api_base: call.api_base,
                stream: call.stream,
                params: VertexParams {
                    project: call.params.vertex_project.as_deref(),
                    location: call.params.vertex_location.as_deref(),
                    ..VertexParams::default()
                },
                vertex: &vertex,
                studio: &studio,
                studio_keys: NativeParams {
                    api_key: call
                        .params
                        .api_key
                        .as_ref()
                        .map(|key| AuthValue::Key(key.clone())),
                    gemini_api_key: None,
                },
            };
            let target = native_auth(self.catalog.as_ref(), &auth, |_credentials, _project| {
                ready(Ok(AccessToken {
                    token: SecretString::from(""),
                    project: String::new(),
                }))
            })
            .await
            .map_err(ConfigError::Native)?;
            Ok(auth_target(target.url, target.headers))
        }
    }

    // transform_generate_content_response: a reply it cannot parse becomes the base config's error class
    fn decode_response(
        &self,
        response: &UpstreamResponse,
    ) -> Result<Self::Response, BaseLlmException> {
        let headers = index_headers(&response.headers);
        transform_response(&RawResponse {
            status: response.status,
            headers: &headers,
            body: &response.body,
        })
        .map(|native| native.response)
        .map_err(|error| {
            self.error_class(
                &format!("Error transforming generate content response: {error}"),
                response.status,
                header_map(&response.headers),
            )
        })
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

// The handler carries the provider's headers as plain strings, where the native auth keeps each value secret
fn auth_target(url: String, headers: BTreeMap<String, SecretString>) -> AuthTarget {
    AuthTarget {
        url,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name, value.expose_secret().to_owned()))
            .collect(),
    }
}

fn index_headers(headers: &HttpHeaders) -> IndexMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}
