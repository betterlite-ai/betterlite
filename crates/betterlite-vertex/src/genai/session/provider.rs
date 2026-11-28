use super::error::GenaiConfigError;
use crate::credential_sources::aws::AwsCredentialsSource;
use crate::credential_sources::files::StdFiles;
use crate::credential_sources::getters::VertexParams;
use crate::credential_sources::handles::{LoadDeps, load_auth};
use crate::credential_sources::sdk::{CredentialClock, GoogleAuth};
use crate::genai::genai_contract::config::TOP_LEVEL_FIELDS;
use crate::google_native_content::auth::{
    AuthTarget as VertexTarget, NativeAuth, auth_token_and_url as vertex_auth,
};
use crate::google_native_content::capabilities::NativeProvider;
use crate::google_native_content::map::{studio_optional_params, vertex_optional_params};
use crate::google_native_content::request::{
    GenerateContentRequest as VertexRequest, transform_request as vertex_transform,
};
use crate::google_native_content::response::{
    GenerateContentResponse, decode_google_native_content,
};
use crate::studio_identity::{AuthValue, NativeParams};
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use betterlite_config::sections::studio_identity::Settings as StudioSettings;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_core::api::genai_http::config::{AuthCall, AuthTarget, GenerateContentHttpConfig};
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap, header_map,
};
use betterlite_core::genai_setup::config::{
    GenerateContentProviderConfig, GenerateContentRequest as SetupRequest,
};
use betterlite_core::genai_setup::params::VertexCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

// LiteLLM's GoogleGenAIConfig and VertexAIGoogleGenAIConfig differ in four methods of one class, so one config serves both slots
pub struct NativeContentConfig {
    pub(super) provider: NativeProvider,
    pub(super) catalog: Arc<ModelCatalog>,
    pub(super) studio: StudioSettings,
    pub(super) vertex: VertexSettings,
    pub(super) schema: SchemaSettings,
    pub(super) google: GoogleAuth,
    pub(super) clock: Arc<dyn CredentialClock>,
    pub(super) aws: Arc<dyn AwsCredentialsSource>,
}

impl GenerateContentProviderConfig for NativeContentConfig {
    type Error = GenaiConfigError;

    // map_generate_content_optional_params: Studio drops what it cannot send and answers camelCase, Vertex keeps every key in snake_case
    fn map_optional_params(&self, config: &Object, _model: &str) -> Object {
        match self.provider {
            NativeProvider::Studio => studio_optional_params(config),
            NativeProvider::Vertex => vertex_optional_params(config),
        }
    }

    fn transform_request(&self, request: &SetupRequest<'_>) -> Result<Json, GenaiConfigError> {
        let native = native_request(request);
        let body = vertex_transform(self.provider, &native, &self.schema)?;
        let encoded =
            serde_json::to_value(body).map_err(|source| GenaiConfigError::Body { source })?;
        serde_json::from_value::<Json>(encoded).map_err(|source| GenaiConfigError::Body { source })
    }

    fn request_top_level_fields(&self) -> &'static [&'static str] {
        &TOP_LEVEL_FIELDS
    }
}

impl GenerateContentHttpConfig for NativeContentConfig {
    type Response = GenerateContentResponse;

    fn auth_token_and_url(
        &self,
        call: &AuthCall<'_>,
    ) -> impl Future<Output = Result<AuthTarget, GenaiConfigError>> + Send {
        let configured = call
            .params
            .vertex_credentials
            .as_ref()
            .map(configured_credentials)
            .transpose();
        async move {
            let resolved = configured?;
            self.target(call, resolved.as_ref()).await
        }
    }

    // transform_generate_content_response: a reply it cannot read becomes the provider's own error class
    fn decode_response(
        &self,
        response: &UpstreamResponse,
    ) -> Result<Self::Response, BaseLlmException> {
        decode_google_native_content(&response.body).map_err(|source| {
            self.error_class(
                &format!("Error transforming generate content response: {source}"),
                response.status,
                header_map(&response.headers),
            )
        })
    }

    // BaseGoogleGenAIGenerateContentConfig.get_error_class keeps the provider's status, message and headers
    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

// the owned inputs the auth lookup needs, so the five it used to take stay inside the clippy limit
struct LookupParts {
    vertex: VertexSettings,
    google: GoogleAuth,
    clock: Arc<dyn CredentialClock>,
    aws: Arc<dyn AwsCredentialsSource>,
}

impl NativeContentConfig {
    // get_auth_token_and_url: the credentials, the project and the location, then the URL and the headers they select
    async fn target(
        &self,
        call: &AuthCall<'_>,
        configured: Option<&ConfiguredCredentials>,
    ) -> Result<AuthTarget, GenaiConfigError> {
        let auth = self.native_auth(call, configured);
        let parts = LookupParts {
            vertex: self.vertex.clone(),
            google: GoogleAuth {
                client: self.google.client.clone(),
                policy: self.google.policy.clone(),
            },
            clock: Arc::clone(&self.clock),
            aws: Arc::clone(&self.aws),
        };
        let resolver = move |credentials: Option<&ConfiguredCredentials>, project: Option<&str>| {
            lookup(parts, credentials.cloned(), project.map(str::to_owned))
        };
        let target = vertex_auth(self.catalog.as_ref(), &auth, resolver).await?;
        let VertexTarget { url, headers } = target;
        Ok(AuthTarget {
            url,
            headers: exposed_headers(headers),
        })
    }

    // LiteLLM's _get_common_auth_components with the per-deployment params it hands the Vertex getters
    fn native_auth<'auth>(
        &'auth self,
        call: &'auth AuthCall<'_>,
        configured: Option<&'auth ConfiguredCredentials>,
    ) -> NativeAuth<'auth> {
        NativeAuth {
            provider: self.provider,
            model: call.model,
            api_base: call.api_base,
            stream: call.stream,
            params: VertexParams {
                project: call.params.vertex_project.as_deref(),
                ai_project: None,
                location: call.params.vertex_location.as_deref(),
                ai_location: None,
                credentials: configured,
                ai_credentials: None,
            },
            vertex: &self.vertex,
            studio: &self.studio,
            studio_keys: NativeParams {
                api_key: call.params.api_key.clone().map(AuthValue::Key),
                gemini_api_key: gemini_key(&call.params.extra),
            },
        }
    }
}

// VertexBase.load_auth: the fresh handle, then the token and the project it resolved
async fn lookup(
    parts: LookupParts,
    credentials: Option<ConfiguredCredentials>,
    project: Option<String>,
) -> Result<AccessToken, CredentialCacheError> {
    let files = StdFiles;
    let deps = LoadDeps {
        environment: &parts.vertex.adc,
        files: &files,
        http: &parts.google,
        aws: parts.aws,
        clock: parts.clock,
    };
    let loaded = load_auth(credentials.as_ref(), project.as_deref(), &deps)
        .await
        .map_err(CredentialCacheError::Load)?;
    let token = loaded
        .handle
        .access_token()
        .await
        .map_err(CredentialCacheError::Load)?;
    Ok(AccessToken {
        token,
        project: loaded.project,
    })
}

fn native_request(request: &SetupRequest<'_>) -> VertexRequest {
    VertexRequest {
        model: request.model.to_owned(),
        contents: request.contents.clone(),
        tools: request.tools.cloned().unwrap_or(Json::Null),
        generation_config: request.generation_config.clone(),
        system_instruction: request.system_instruction.cloned().unwrap_or(Json::Null),
    }
}

// _get_google_ai_studio_api_key reads gemini_api_key after api_key and before the environment, so it cannot be a params field
pub(super) fn gemini_key(extra: &Object) -> Option<AuthValue> {
    match extra.get("gemini_api_key") {
        Some(Json::Text(key)) => Some(AuthValue::Key(SecretString::from(key.clone()))),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// LiteLLM's credentials value is a service-account document, a filename or JSON text, and from_text reads all three
fn configured_credentials(
    credentials: &VertexCredentials,
) -> Result<ConfiguredCredentials, GenaiConfigError> {
    let text = match credentials {
        VertexCredentials::Path(path) => path.clone(),
        VertexCredentials::ServiceAccount(account) => {
            serde_json::to_string(account).map_err(|source| GenaiConfigError::Body { source })?
        }
    };
    ConfiguredCredentials::from_text(text)
        .map_err(|source| GenaiConfigError::Credentials { source })
}

// the provider answers secrets; the wire takes text
fn exposed_headers(headers: BTreeMap<String, SecretString>) -> IndexMap<String, String> {
    headers
        .into_iter()
        .map(|(name, value)| (name, value.expose_secret().to_owned()))
        .collect()
}
