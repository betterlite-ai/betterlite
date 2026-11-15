use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_schema_shaping::{Raw, Settings};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json as ResponseJson;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::gemini_image_policy::generation::image_web_search_requests;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::vertex_gemini_images::options::{MappedOptions, map_openai_params};
use betterlite_vertex::vertex_gemini_images::request::transform_image_generation_request;
use betterlite_vertex::vertex_gemini_images::response::transform_image_generation_response;
use betterlite_vertex::vertex_image_identity::settings::{ImageEnvironment, ImageIdentitySettings};
use betterlite_vertex::vertex_image_identity::target::{
    ImageTarget, ValidationRequest, complete_url, validate_environment,
};
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) model: String,
    pub(super) mappings: Vec<Mapping>,
    pub(super) target: Option<Target>,
    pub(super) response: Option<IndexMap<String, ResponseJson>>,
}

// the arguments of VertexAIGeminiImageGenerationConfig.map_openai_params
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mapping {
    prompt: String,
    non_default_params: Object,
    optional_params: Object,
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
    let settings = Raw::default()
        .convert(&BTreeMap::<String, String>::new())
        .or_config_error()?;
    let mut mappings = Vec::with_capacity(input.mappings.len());
    for mapping in &input.mappings {
        mappings.push(mapping_value(mapping, &settings)?);
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

fn mapping_value(mapping: &Mapping, settings: &Settings) -> Result<Value, Box<HelperError>> {
    let MappedOptions { params, warnings } = map_openai_params(
        &mapping.non_default_params,
        &mapping.optional_params,
        settings,
    )
    .or_value_error()?;
    let body = transform_image_generation_request(&mapping.prompt, &params).or_value_error()?;
    Ok(json!({
        "mapped": params,
        "warnings": warnings,
        "body": body,
    }))
}

fn target_value(model: &str, target: &Target) -> Result<Value, Box<HelperError>> {
    let settings = target_settings(target);
    let url = complete_url(
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
        environment: ImageEnvironment {
            project: target.project.clone(),
            location: target.location.clone(),
            ..ImageEnvironment::default()
        },
        ..ImageIdentitySettings::default()
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
    // the cases that reach this helper use a custom base, so no token is ever minted here
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

fn reply_value(reply: &IndexMap<String, ResponseJson>) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_vec(reply).or_config_error()?;
    let response = transform_image_generation_response(&UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body,
    })
    .or_value_error()?;
    Ok(json!({
        "response": serde_json::to_value(response).or_config_error()?,
        "search_requests": image_web_search_requests(reply),
    }))
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

// the custom-base cases never mint a token, so a load or a refresh that runs fails the case
struct NoCredentials;

impl CredentialExecutor for NoCredentials {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(ready(Err(CredentialCacheError::MissingCredentials)))
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(ready(Err(CredentialCacheError::MissingToken)))
    }
}
