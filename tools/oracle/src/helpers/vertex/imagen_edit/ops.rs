use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_schema_shaping::Raw;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_vertex::credential_sources::getters::VertexParams;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::vertex_image_identity::settings::ImageIdentitySettings;
use betterlite_vertex::vertex_image_identity::target::{
    ImageTarget, ValidationRequest, validate_environment,
};
use betterlite_vertex::vertex_imagen_edit::input::{ImagenFields, ImagenFileInput};
use betterlite_vertex::vertex_imagen_edit::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_vertex::vertex_imagen_edit::request::transform_image_edit_request;
use betterlite_vertex::vertex_imagen_edit::response::transform_image_edit_response;
use betterlite_vertex::vertex_imagen_edit::target::predict_url;
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::runtime::Builder;

// the depth setting is B1307's section, read from the same variable the Python helper's import reads
const DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH";
// the reply's created is a wall-clock stamp in LiteLLM, so cases ignore it and the helper pins it
const CREATED: i64 = 0;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) model: String,
    pub(super) prompt: Option<String>,
    pub(super) image: Option<Vec<Option<ImageSpec>>>,
    pub(super) mask: Option<ImageSpec>,
    pub(super) optional_params: ImageParams,
    pub(super) target: Option<Target>,
    pub(super) response: Option<IndexMap<String, Value>>,
    pub(super) max_recurse_depth: Option<i64>,
}

// the file records the edit reader accepts, plus the wrappers and rejections its cases exercise
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ImageSpec {
    Bytes {
        data_base64: String,
    },
    Text {
        text: String,
    },
    // a nested list counts one level toward the reader's depth limit, so the case can hand it more
    Sequence {
        items: Vec<Option<Self>>,
    },
    Mapping {
        data: Option<String>,
        bytes: Option<String>,
        content: Option<String>,
        path: Option<String>,
    },
}

// the values get_complete_url and validate_environment read
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
    let mapped = map_openai_params(&input.optional_params).or_value_error()?;
    let mut images = image_list(input.image.as_deref())?;
    let mut mask = input.mask.as_ref().map(image_input).transpose()?;
    let max_depth = depth_limit(input)?;
    let (payload, files) = transform_image_edit_request(
        input.prompt.as_deref(),
        images.as_deref_mut(),
        mask.as_mut(),
        &mapped,
        max_depth,
    )
    .or_value_error()?;
    let target = input
        .target
        .as_ref()
        .map_or(Ok(Value::Null), |target| target_value(&input.model, target))?;
    let response = input
        .response
        .as_ref()
        .map_or(Ok(Value::Null), reply_value)?;
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
        "payload": serde_json::from_str::<Value>(&payload).or_config_error()?,
        "files": files.len(),
        "target": target,
        "response": response,
    }))
}

// the section reads the variable a case sets in its environment, and an unset one keeps the shared default
fn depth_limit(input: &Input) -> Result<i64, Box<HelperError>> {
    let mut environment = BTreeMap::new();
    if let Some(depth) = input.max_recurse_depth {
        environment.insert(DEPTH_VARIABLE.to_owned(), depth.to_string());
    }
    let settings = Raw::default().convert(&environment).or_config_error()?;
    Ok(settings.max_recurse_depth)
}

fn image_list(
    specs: Option<&[Option<ImageSpec>]>,
) -> Result<Option<Vec<Option<ImagenFileInput<'static>>>>, Box<HelperError>> {
    let Some(entries) = specs else {
        return Ok(None);
    };
    let mut images = Vec::with_capacity(entries.len());
    for spec in entries {
        images.push(spec.as_ref().map(image_input).transpose()?);
    }
    Ok(Some(images))
}

fn image_input(spec: &ImageSpec) -> Result<ImagenFileInput<'static>, Box<HelperError>> {
    match spec {
        ImageSpec::Bytes { data_base64 } => Ok(ImagenFileInput::Bytes(
            STANDARD.decode(data_base64).or_config_error()?,
        )),
        ImageSpec::Text { text } => Ok(ImagenFileInput::Text(text.clone())),
        ImageSpec::Sequence { items } => {
            let mut members = Vec::with_capacity(items.len());
            for item in items {
                members.push(item.as_ref().map(image_input).transpose()?);
            }
            Ok(ImagenFileInput::Sequence(members))
        }
        ImageSpec::Mapping {
            data,
            bytes,
            content,
            path,
        } => Ok(ImagenFileInput::Mapping(mapping_input(
            data.as_deref(),
            bytes.as_deref(),
            content.as_deref(),
            path.as_deref(),
        ))),
    }
}

// a dict's value keys hold text the reader decodes as base64, and its path holds text it refuses
fn mapping_input(
    data: Option<&str>,
    bytes: Option<&str>,
    content: Option<&str>,
    path: Option<&str>,
) -> ImagenFields<'static> {
    let mut fields = ImagenFields {
        data: None,
        bytes: None,
        content: None,
        path: None,
    };
    if let Some(data_value) = data {
        fields.data = Some(Box::new(ImagenFileInput::Text(data_value.to_owned())));
    }
    if let Some(bytes_value) = bytes {
        fields.bytes = Some(Box::new(ImagenFileInput::Text(bytes_value.to_owned())));
    }
    if let Some(content_value) = content {
        fields.content = Some(Box::new(ImagenFileInput::Text(content_value.to_owned())));
    }
    if let Some(path_value) = path {
        fields.path = Some(Box::new(ImagenFileInput::Path(PathBuf::from(path_value))));
    }
    fields
}

fn target_value(model: &str, target: &Target) -> Result<Value, Box<HelperError>> {
    let settings = ImageIdentitySettings::default();
    let params = VertexParams {
        project: target.project.as_deref(),
        ai_project: None,
        location: target.location.as_deref(),
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    let url = predict_url(
        &settings,
        &ImageTarget {
            model,
            api_base: target.api_base.as_deref(),
            params,
        },
    )
    .or_value_error()?;
    let headers = target_headers(target, &settings, params)?;
    Ok(json!({"url": url, "headers": headers}))
}

fn target_headers(
    target: &Target,
    settings: &ImageIdentitySettings,
    params: VertexParams<'_>,
) -> Result<BTreeMap<String, String>, Box<HelperError>> {
    let request = ValidationRequest {
        params,
        api_base: target.api_base.as_deref(),
        params_api_base: target.params_api_base.as_deref(),
        headers: &target.headers,
    };
    // the custom-base cases never mint a token, so a load or a refresh that runs fails the case
    let cache = CredentialCache::new(Arc::new(FixedClock), Arc::new(NoCredentials));
    let runtime = Builder::new_current_thread().build().or_config_error()?;
    let headers = runtime
        .block_on(validate_environment(&cache, settings, &request))
        .or_value_error()?;
    Ok(headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect())
}

fn reply_value(reply: &IndexMap<String, Value>) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_vec(reply).or_config_error()?;
    let response = transform_image_edit_response(
        &UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body,
        },
        CREATED,
    )
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
