use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::api::image_input_options::mime::image_content_type_stream;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::vertex_gemini_edit::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_vertex::vertex_gemini_edit::request::{
    GeminiEditFile, transform_image_edit_request,
};
use betterlite_vertex::vertex_gemini_edit::response::transform_image_edit_response;
use betterlite_vertex::vertex_image_identity::settings::ImageIdentitySettings;
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
    pub(super) prompt: Option<String>,
    pub(super) image: Option<Vec<Option<ImageSpec>>>,
    pub(super) optional_params: ImageParams,
    pub(super) target: Option<Target>,
    pub(super) response: Option<IndexMap<String, Value>>,
}

// the file record variants the edit reader accepts, plus one it rejects
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ImageSpec {
    Bytes { data_base64: String },
    Seekable { data_base64: String, position: u64 },
    Unsupported,
}

// the values get_complete_url and validate_environment read
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    api_base: Option<String>,
    params_api_base: Option<String>,
    headers: BTreeMap<String, String>,
}

pub(super) fn run_cases(input: &Input) -> HelperResult {
    let mapped = map_openai_params(&input.optional_params).or_value_error()?;
    let mut prepared = prepared_images(input.image.as_deref())?;
    let mut views: Vec<Option<GeminiEditFile<'_>>> = Vec::with_capacity(prepared.len());
    for slot in &mut prepared {
        views.push(match slot {
            Some(PreparedImage::Bytes(content)) => Some(GeminiEditFile::Bytes(content.as_slice())),
            Some(PreparedImage::Seekable(cursor)) => Some(GeminiEditFile::Seekable(cursor)),
            Some(PreparedImage::Unsupported) => Some(GeminiEditFile::Unsupported),
            None => None,
        });
    }
    // B033 injects the same core sniffer, so the oracle compares its answer with LiteLLM's reader
    let sniff = |content: &[u8]| image_content_type_stream(&mut Cursor::new(content));
    let (payload, files) =
        transform_image_edit_request(input.prompt.as_deref(), &mut views, &mapped, &sniff)
            .or_value_error()?;
    drop(views);
    let positions = positions_value(&prepared);
    let target = match &input.target {
        Some(target) => target_value(&input.model, target)?,
        None => Value::Null,
    };
    let response = match &input.response {
        Some(reply) => reply_value(reply)?,
        None => Value::Null,
    };
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
        "payload": payload,
        "files": files.len(),
        "positions": positions,
        "target": target,
        "response": response,
    }))
}

enum PreparedImage {
    Bytes(Vec<u8>),
    Seekable(Cursor<Vec<u8>>),
    Unsupported,
}

fn prepared_images(
    specs: Option<&[Option<ImageSpec>]>,
) -> Result<Vec<Option<PreparedImage>>, Box<HelperError>> {
    let Some(specs) = specs else {
        return Ok(Vec::new());
    };
    let mut prepared = Vec::with_capacity(specs.len());
    for spec in specs {
        let record = match spec {
            Some(spec) => Some(prepared_image(spec)?),
            None => None,
        };
        prepared.push(record);
    }
    Ok(prepared)
}

fn prepared_image(spec: &ImageSpec) -> Result<PreparedImage, Box<HelperError>> {
    match spec {
        ImageSpec::Bytes { data_base64 } => Ok(PreparedImage::Bytes(
            STANDARD.decode(data_base64).or_config_error()?,
        )),
        ImageSpec::Seekable {
            data_base64,
            position,
        } => {
            let mut cursor = Cursor::new(STANDARD.decode(data_base64).or_config_error()?);
            cursor.set_position(*position);
            Ok(PreparedImage::Seekable(cursor))
        }
        ImageSpec::Unsupported => Ok(PreparedImage::Unsupported),
    }
}

// the cursor each record reads from after the transform; only a seekable record has one
fn positions_value(prepared: &[Option<PreparedImage>]) -> Value {
    let mut positions = Vec::with_capacity(prepared.len());
    for slot in prepared {
        let position = match slot {
            Some(PreparedImage::Seekable(cursor)) => json!(cursor.position()),
            Some(PreparedImage::Bytes(_) | PreparedImage::Unsupported) | None => Value::Null,
        };
        positions.push(position);
    }
    Value::Array(positions)
}

fn target_value(model: &str, target: &Target) -> Result<Value, Box<HelperError>> {
    let settings = ImageIdentitySettings::default();
    let url = complete_url(
        &settings,
        &ImageTarget {
            model,
            api_base: target.api_base.as_deref(),
            params: no_params(),
        },
    )
    .or_value_error()?;
    let headers = target_headers(target, &settings)?;
    Ok(json!({"url": url, "headers": headers}))
}

fn target_headers(
    target: &Target,
    settings: &ImageIdentitySettings,
) -> Result<BTreeMap<String, String>, Box<HelperError>> {
    let request = ValidationRequest {
        params: no_params(),
        api_base: target.api_base.as_deref(),
        params_api_base: target.params_api_base.as_deref(),
        headers: &target.headers,
    };
    // the custom-base case never mints a token, so a load or a refresh that runs fails the case
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

const fn no_params() -> VertexParams<'static> {
    VertexParams {
        project: None,
        ai_project: None,
        location: None,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    }
}

fn reply_value(reply: &IndexMap<String, Value>) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_vec(reply).or_config_error()?;
    let response = transform_image_edit_response(&UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body,
    })
    .or_value_error()?;
    serde_json::to_value(response).or_config_error()
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
