mod credentials;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_openai::api::image_contracts::records::ImageFiles;
use betterlite_openai::image_edits::environment::{complete_url, validate_environment};
use betterlite_openai::image_edits::error::ImageEditError;
use betterlite_openai::image_edits::family::{SUPPORTED_OPENAI_PARAMS, select_image_edit_family};
use betterlite_openai::image_edits::file::{ImageEditFile, multipart_parts};
use betterlite_openai::image_edits::request::{ImageEditRequest, transform_image_edit_request};
use betterlite_openai::image_edits::response::transform_image_edit_response;
use betterlite_openai::openai_base_config::Headers;
use http::HeaderMap;
use serde_json::{Map, Value, json};

use super::Input;
use super::input::{FileSpec, Probe};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

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
        ImageEditError::MultipleImages => "BadRequestError",
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

fn file_of(spec: &FileSpec) -> Result<ImageEditFile, Box<HelperError>> {
    let content = BASE64
        .decode(&spec.content_base64)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(ImageEditFile {
        content,
        name: spec.name.clone(),
    })
}

fn image_files(
    specs: Option<&[Option<FileSpec>]>,
) -> Result<Option<Vec<Option<ImageEditFile>>>, Box<HelperError>> {
    let Some(supplied) = specs else {
        return Ok(None);
    };
    let files = supplied
        .iter()
        .map(|spec| spec.as_ref().map(file_of).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(files))
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
        ImageEditError::MultipleImages => "bad_request",
        ImageEditError::OpenAi(_) => "openai_error",
        ImageEditError::Reply(_) => "validation",
        ImageEditError::ApiBase(_)
        | ImageEditError::HeaderName(_)
        | ImageEditError::HeaderValue(_)
        | ImageEditError::Authorization(_) => "error",
    };
    json!({"kind": kind, "status": error.status()})
}

fn request_of(probe: &Probe) -> Result<ImageEditRequest, Box<HelperError>> {
    let image = image_files(probe.image.as_deref())?;
    let mask = image_files(probe.mask.as_deref())?;
    Ok(ImageEditRequest {
        model: probe.model.clone(),
        prompt: probe.prompt.clone(),
        image,
        mask,
        options: probe.options.clone(),
    })
}

pub(crate) fn values(input: &Input) -> HelperResult {
    let values = match input {
        Input::Transform(probes) => probes
            .iter()
            .map(transform_value)
            .collect::<Result<Vec<_>, Box<HelperError>>>()?,
        Input::Reply(probes) => probes
            .iter()
            .map(reply_value)
            .collect::<Result<Vec<_>, Box<HelperError>>>()?,
    };
    Ok(Value::Array(values))
}

fn transform_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let resolved = credentials::resolve(probe)?;
    let call_url = complete_url(&resolved.api_base).map_err(|error| failure(&error))?;
    let call_headers =
        validate_environment(&Headers::new(), &resolved).map_err(|error| failure(&error))?;
    let family = select_image_edit_family(&probe.model);
    let (url, headers, data, files, error) =
        match transform_image_edit_request(family, &request_of(probe)?) {
            Ok(form) => {
                let data =
                    serde_json::to_value(&form.data).map_err(|error| invalid(error.to_string()))?;
                (
                    Value::String(call_url.as_str().to_owned()),
                    header_values(&call_headers),
                    data,
                    files_value(&form.files),
                    Value::Null,
                )
            }
            Err(error) => (
                Value::Null,
                Value::Null,
                Value::Null,
                Value::Null,
                error_value(&error),
            ),
        };
    Ok(json!({
        "family": family.name(),
        "supported": SUPPORTED_OPENAI_PARAMS,
        "url": url,
        "headers": headers,
        "data": data,
        "files": files,
        "error": error,
    }))
}

fn reply_value(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let reply = probe
        .reply
        .as_ref()
        .ok_or_else(|| invalid(String::from("a probe needs a reply")))?;
    let transformed = transform_image_edit_response(reply.status, reply.body.as_bytes(), NOW)
        .map_err(|error| failure(&error))?;
    serde_json::to_value(&transformed).map_err(|error| invalid(error.to_string()))
}
