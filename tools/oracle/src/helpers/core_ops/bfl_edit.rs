use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::RouterSettings;
use betterlite_core::api::bfl::common::BflError;
use betterlite_core::api::bfl::edit::media::{MaskInput, SuppliedFile, SuppliedMedia};
use betterlite_core::api::bfl::edit::{
    EditError, SUPPORTED_OPENAI_PARAMS, complete_url, map_openai_params,
    transform_image_edit_request, transform_image_edit_response,
};
use betterlite_core::api::bfl::generation::validate_environment;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::ErrorCode;
use indexmap::IndexMap;
use reqwest::header::HeaderMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, json};

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum MediaSpec {
    Bytes { data: String },
    File { data: String, position: usize },
    Text { text: String },
    List { items: Vec<Self> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum MaskSpec {
    Absent,
    Null,
    Supplied { media: MediaSpec },
}

#[derive(Debug, Deserialize)]
pub(crate) struct PrepareParams {
    prompt: Option<String>,
    image: Option<MediaSpec>,
    mask: MaskSpec,
    options: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DefaultsParams {
    model: String,
    drop_params: bool,
    options: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndpointParams {
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SampleParams {
    status: u16,
    body: String,
    now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    FirstImageOnly(PrepareParams),
    RestoredFileCursor(PrepareParams),
    MaskPresence(PrepareParams),
    EditDefaults(DefaultsParams),
    EndpointFill(EndpointParams),
    SampleRequired(SampleParams),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::FirstImageOnly(params)
            | Input::RestoredFileCursor(params)
            | Input::MaskPresence(params) => prepare(params),
            Input::EditDefaults(params) => {
                let mapped = map_openai_params(&params.options, &params.model, params.drop_params);
                Ok(json!({"supported": SUPPORTED_OPENAI_PARAMS, "mapped": mapped}))
            }
            Input::EndpointFill(params) => endpoint(params),
            Input::SampleRequired(params) => sample(params),
        }
    }
}

fn bfl_failure(error: &BflError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BflError".to_owned(),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn edit_failure(error: &EditError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "EditError".to_owned(),
        status: error.status(),
        error_type: error.error_type().map(|text| Some(text.to_owned())),
        param: error.param().map(|text| Some(text.to_owned())),
        code: error
            .code()
            .map(|text| Some(ErrorCode::Text(text.to_owned()))),
        message: error.to_string(),
    })
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn decoded(text: &str) -> Result<Vec<u8>, Box<HelperError>> {
    BASE64
        .decode(text)
        .map_err(|error| invalid(error.to_string()))
}

fn media_from(spec: &MediaSpec) -> Result<SuppliedMedia, Box<HelperError>> {
    Ok(match spec {
        MediaSpec::Bytes { data } => SuppliedMedia::Bytes(decoded(data)?),
        MediaSpec::File { data, position } => SuppliedMedia::File(SuppliedFile {
            content: decoded(data)?,
            position: *position,
        }),
        MediaSpec::Text { text } => SuppliedMedia::Text(text.clone()),
        MediaSpec::List { items } => SuppliedMedia::List(
            items
                .iter()
                .map(media_from)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    })
}

fn positions(media: &SuppliedMedia) -> Vec<usize> {
    match media {
        SuppliedMedia::File(file) => vec![file.position],
        SuppliedMedia::List(items) => items.iter().flat_map(positions).collect(),
        SuppliedMedia::Bytes(_) | SuppliedMedia::Text(_) => Vec::new(),
    }
}

// LiteLLM's async override fetches a remote first or mask candidate before transforming; this build refuses the URL instead
fn prepare(params: &PrepareParams) -> HelperResult {
    // DEFAULT_MAX_RECURSE_DEPTH enters through the loader; a clean environment reads the default of 100
    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .or_config_error()?;
    let max_depth = loaded.sections.mistral_options.max_recurse_depth;
    let mut image = params.image.as_ref().map(media_from).transpose()?;
    let mut mask = match &params.mask {
        MaskSpec::Absent | MaskSpec::Null => None,
        MaskSpec::Supplied { media } => Some(media_from(media)?),
    };
    let mask_input = match &params.mask {
        MaskSpec::Null => MaskInput::Null,
        MaskSpec::Absent | MaskSpec::Supplied { .. } => {
            mask.as_mut().map_or(MaskInput::Absent, MaskInput::Media)
        }
    };
    let form = transform_image_edit_request(
        params.prompt.as_deref(),
        image.as_mut(),
        mask_input,
        &params.options,
        max_depth,
    )
    .map_err(|error| edit_failure(&error))?;
    let mut after = image.as_ref().map_or_else(Vec::new, positions);
    if let Some(media) = &mask {
        after.extend(positions(media));
    }
    Ok(json!({"data": form.data, "files": form.files.len(), "positions": after}))
}

fn endpoint(params: &EndpointParams) -> HelperResult {
    let (configured_key, configured_base) =
        RouterSettings::bfl_generation_settings(&params.environment);
    let key = params.api_key.clone().map(SecretString::from);
    let prepared = validate_environment(HeaderMap::new(), key.as_ref(), configured_key.as_ref())
        .map_err(|error| bfl_failure(&error))?;
    let mut headers = Map::new();
    for (name, value) in &prepared {
        let text = value.to_str().map_err(|error| invalid(error.to_string()))?;
        headers.insert(name.as_str().to_owned(), json!(text));
    }
    let url = complete_url(params.api_base.as_deref(), &params.model, &configured_base)
        .map_err(|error| bfl_failure(&error))?;
    Ok(json!({"headers": headers, "url": url}))
}

fn sample(params: &SampleParams) -> HelperResult {
    let response = transform_image_edit_response(params.status, params.body.as_bytes(), params.now)
        .map_err(|error| edit_failure(&error))?;
    let data = response
        .data
        .iter()
        .map(|datum| json!({"url": datum.url}))
        .collect::<Vec<_>>();
    Ok(json!({"created": response.created, "data": data}))
}
