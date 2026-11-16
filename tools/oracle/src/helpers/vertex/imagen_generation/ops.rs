use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::api::image_contracts::records::ImageUsage;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::vertex_image_identity::settings::{
    ImageEnvironment, ImageIdentitySettings, ImageInstance, ImageModule, ImageSecrets,
};
use betterlite_vertex::vertex_image_identity::target::{
    ImageTarget, ValidationRequest, validate_environment,
};
use betterlite_vertex::vertex_imagen_generation::options::map_openai_params;
// ast-grep-ignore: no-long-names
use betterlite_vertex::vertex_imagen_generation::request::transform_image_generation_request;
use betterlite_vertex::vertex_imagen_generation::response::ImagenGenerationResponse;
// ast-grep-ignore: no-long-names
use betterlite_vertex::vertex_imagen_generation::response::transform_image_generation_response;
use betterlite_vertex::vertex_imagen_generation::target::predict_url;
use http::header::HeaderMap;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) model: String,
    pub(super) mappings: Vec<Mapping>,
    pub(super) target: Option<Target>,
    pub(super) response: Option<Extra>,
}

// the arguments VertexAIImagenImageGenerationConfig.map_openai_params and the request transform read
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mapping {
    prompt: String,
    non_default_params: Extra,
    optional_params: Extra,
    litellm_params: Option<Json>,
}

// the values get_complete_url and validate_environment read, plus the environment they fall back to
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    api_base: Option<String>,
    params_api_base: Option<String>,
    project: Option<String>,
    location: Option<String>,
    headers: BTreeMap<String, String>,
}

pub(super) fn run_cases(input: &Input) -> HelperResult {
    let mut mappings = Vec::with_capacity(input.mappings.len());
    for mapping in &input.mappings {
        mappings.push(mapping_value(mapping)?);
    }
    let target = match &input.target {
        Some(target) => target_value(&input.model, target)?,
        None => Value::Null,
    };
    let response = match &input.response {
        Some(reply) => reply_value(reply)?,
        None => Value::Null,
    };
    Ok(json!({
        "mappings": mappings,
        "target": target,
        "response": response,
    }))
}

// get_optional_params_image_gen keeps the mapper's answer alone, and the transform reads that answer
fn mapping_value(mapping: &Mapping) -> Result<Value, Box<HelperError>> {
    let mapped = map_openai_params(&mapping.non_default_params, &mapping.optional_params)
        .or_value_error()?;
    // ast-grep-ignore: no-long-names
    let body = transform_image_generation_request(
        &mapping.prompt,
        &mapped,
        mapping.litellm_params.as_ref(),
    );
    Ok(json!({"mapped": mapped, "body": body}))
}

fn target_value(model: &str, target: &Target) -> Result<Value, Box<HelperError>> {
    let settings = target_settings(target);
    let url = predict_url(
        &settings,
        &ImageTarget {
            model,
            api_base: target.api_base.as_deref(),
            params: VertexParams::default(),
        },
    )
    .or_value_error()?;
    let headers = target_headers(target, &settings)?;
    Ok(json!({"url": url, "headers": headers}))
}

fn target_settings(target: &Target) -> ImageIdentitySettings {
    ImageIdentitySettings {
        instance: ImageInstance {
            project: None,
            location: None,
            credentials: None,
        },
        environment: ImageEnvironment {
            project: target.project.clone(),
            location: target.location.clone(),
            legacy_location: None,
            credentials: None,
            application_credentials: None,
        },
        module: ImageModule {
            project: None,
            location: None,
            credentials: None,
        },
        secrets: ImageSecrets {
            project: None,
            location: None,
            legacy_location: None,
            credentials: None,
        },
    }
}

fn target_headers(
    target: &Target,
    settings: &ImageIdentitySettings,
) -> Result<BTreeMap<String, String>, Box<HelperError>> {
    let request = ValidationRequest {
        params: VertexParams::default(),
        api_base: target.api_base.as_deref(),
        params_api_base: target.params_api_base.as_deref(),
        headers: &target.headers,
    };
    // the cases that reach this helper use a custom or empty base, so no token is ever minted here
    let cache = CredentialCache::new(Arc::new(FixedClock), Arc::new(NoCredentials));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .or_config_error()?;
    let headers = runtime
        .block_on(validate_environment(&cache, settings, &request))
        .or_value_error()?;
    Ok(headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect())
}

fn reply_value(reply: &Extra) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_vec(reply).or_config_error()?;
    // ImageResponse() fills in a zero usage, and the transform leaves that usage exactly as it found it
    let mut response = ImagenGenerationResponse {
        data: Vec::new(),
        usage: Some(ImageUsage::default()),
    };
    // ast-grep-ignore: no-long-names
    transform_image_generation_response(
        &UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body,
        },
        &mut response,
    )
    .or_value_error()?;
    Ok(json!({
        "response": serde_json::to_value(response).or_config_error()?,
    }))
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

type CredentialFuture<'borrow, Loaded> =
    Pin<Box<dyn Future<Output = Result<Loaded, CredentialCacheError>> + Send + 'borrow>>;

// the custom-base cases never mint a token, so a load or a refresh that runs fails the case
struct NoCredentials;

impl CredentialExecutor for NoCredentials {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> CredentialFuture<'borrow, LoadedCredential> {
        Box::pin(ready(Err(CredentialCacheError::MissingCredentials)))
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> CredentialFuture<'borrow, CredentialSnapshot> {
        Box::pin(ready(Err(CredentialCacheError::MissingToken)))
    }
}
