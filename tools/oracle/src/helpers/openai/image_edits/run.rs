use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use betterlite_openai::credentials::{
    ExplicitCredentials, LiteLlmGlobals, OpenAiCredentials, get_openai_credentials,
};
use betterlite_openai::image_edits::environment::{complete_url, validate_environment};
use betterlite_openai::image_edits::error::ImageEditError;
use betterlite_openai::image_edits::family::{SUPPORTED_OPENAI_PARAMS, select_image_edit_family};
use betterlite_openai::image_edits::file::{ImageEditFile, multipart_parts};
use betterlite_openai::image_edits::request::{ImageEditRequest, transform_image_edit_request};
use betterlite_openai::image_edits::response::transform_image_edit_response;
use betterlite_openai::openai_base_config::Headers;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::{Map, Value, json};

use super::input::{FileSpec, Probe, Reply};
use crate::helpers::{HelperError, HelperResult};

const NOW: i64 = 1_700_000_000;

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(error: &ImageEditError) -> Box<HelperError> {
    let class = match error {
        ImageEditError::MultipleImages { .. } => "BadRequestError",
        ImageEditError::OpenAi(_) => "OpenAIError",
        ImageEditError::Reply(_) => "ValidationError",
        ImageEditError::ApiBase(_)
        | ImageEditError::HeaderName(_)
        | ImageEditError::HeaderValue(_)
        | ImageEditError::Authorization(_) => "ImageEditError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn file_of(spec: &Option<FileSpec>) -> Result<Option<ImageEditFile>, Box<HelperError>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let content = BASE64
        .decode(&spec.content_base64)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(Some(ImageEditFile {
        content,
        name: spec.name.clone(),
    }))
}

fn image_files(
    specs: Option<&Vec<Option<FileSpec>>>,
) -> Result<Option<Vec<Option<ImageEditFile>>>, Box<HelperError>> {
    let Some(specs) = specs else {
        return Ok(None);
    };
    let files = specs.iter().map(file_of).collect::<Result<Vec<_>, _>>()?;
    Ok(Some(files))
}

fn credentials(probe: &Probe) -> Result<OpenAiCredentials, Box<HelperError>> {
    let settings = openai_credentials::Raw::default()
        .convert(&probe.environment)
        .map_err(|error| invalid(error.to_string()))?;
    let explicit = ExplicitCredentials {
        api_base: probe.params.api_base.clone(),
        api_key: probe.params.api_key.clone().map(SecretString::from),
        organization: probe.params.organization.clone(),
    };
    let globals = LiteLlmGlobals {
        api_base: probe.globals.api_base.clone(),
        api_key: probe.globals.api_key.clone().map(SecretString::from),
        openai_key: probe.globals.openai_key.clone().map(SecretString::from),
        organization: None,
    };
    let environment_key = probe
        .environment
        .get("OPENAI_API_KEY")
        .filter(|text| !text.is_empty())
        .cloned()
        .map(SecretString::from);
    Ok(get_openai_credentials(
        explicit,
        globals,
        &settings,
        environment_key,
    ))
}

fn header_values(headers: &HeaderMap) -> Value {
    let mut fields = Map::new();
    for (name, value) in headers {
        let Ok(text) = value.to_str() else {
            continue;
        };
        fields.insert(String::from(name.as_str()), Value::String(text.to_owned()));
    }
    Value::Object(fields)
}

fn files_value(files: &ImageFiles) -> Value {
    Value::Array(
        multipart_parts(files)
            .into_iter()
            .map(|part| {
                json!({
                    "name": part.name,
                    "filename": part.file_name,
                    "content_type": part.content_type,
                    "content_base64": BASE64.encode(&part.data),
                })
            })
            .collect(),
    )
}

fn error_value(error: &ImageEditError) -> Value {
    let kind = match error {
        ImageEditError::MultipleImages { .. } => "bad_request",
        ImageEditError::OpenAi(_) => "openai_error",
        ImageEditError::Reply(_) => "validation",
        ImageEditError::ApiBase(_)
        | ImageEditError::HeaderName(_)
        | ImageEditError::HeaderValue(_)
        | ImageEditError::Authorization(_) => "error",
    };
    json!({"kind": kind, "status": error.status()})
}

struct Found {
    family: &'static str,
    data: Value,
    files: Value,
    url: Value,
    headers: Value,
    error: Value,
}

fn request_value(probe: &Probe) -> Result<Found, Box<HelperError>> {
    let resolved = credentials(probe)?;
    let family = select_image_edit_family(&probe.model);
    let url = complete_url(&resolved.api_base).map_err(|error| failure(&error))?;
    let headers =
        validate_environment(&Headers::new(), &resolved).map_err(|error| failure(&error))?;
    let request = ImageEditRequest {
        model: probe.model.clone(),
        prompt: probe.prompt.clone(),
        image: image_files(probe.image.as_ref())?,
        mask: image_files(probe.mask.as_ref())?,
        options: probe.options.clone(),
    };
    let mut found = Found {
        family: family.name(),
        data: Value::Null,
        files: Value::Null,
        url: Value::String(url.as_str().to_owned()),
        headers: header_values(&headers),
        error: Value::Null,
    };
    match transform_image_edit_request(family, &request) {
        Ok(form) => {
            found.data =
                serde_json::to_value(&form.data).map_err(|error| invalid(error.to_string()))?;
            found.files = files_value(&form.files);
        }
        Err(error) => {
            found.url = Value::Null;
            found.headers = Value::Null;
            found.error = error_value(&error);
        }
    }
    Ok(found)
}

fn reply_of(probe: &Probe) -> Result<&Reply, Box<HelperError>> {
    probe
        .reply
        .as_ref()
        .ok_or_else(|| invalid(String::from("a probe needs a reply")))
}

pub(crate) fn transform_values(probes: &[Probe]) -> HelperResult {
    let values = probes
        .iter()
        .map(|probe| {
            let found = request_value(probe)?;
            Ok(json!({
                "family": found.family,
                "supported": SUPPORTED_OPENAI_PARAMS,
                "data": found.data,
                "files": found.files,
                "error": found.error,
            }))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(Value::Array(values))
}

pub(crate) fn environment_values(probes: &[Probe]) -> HelperResult {
    let values = probes
        .iter()
        .map(|probe| {
            let found = request_value(probe)?;
            Ok(json!({
                "url": found.url,
                "headers": found.headers,
                "error": found.error,
            }))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(Value::Array(values))
}

pub(crate) fn reply_values(probes: &[Probe]) -> HelperResult {
    let values = probes
        .iter()
        .map(|probe| {
            let reply = reply_of(probe)?;
            match transform_image_edit_response(reply.status, reply.body.as_bytes(), NOW) {
                Ok(transformed) => {
                    serde_json::to_value(&transformed).map_err(|error| invalid(error.to_string()))
                }
                Err(error) => Err(failure(&error)),
            }
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(Value::Array(values))
}
