use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::openai_video_identifiers::multipart::{MediaBody, MultipartPart};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::{
    MULTIPART_FORM_DATA, SUPPORTED_PARAMS, VideoCredentials, complete_url, validate_environment,
};
use betterlite_openai::videos::content::{content_request, content_response};
use betterlite_openai::videos::create::{create_request, create_response};
use betterlite_openai::videos::delete::{delete_request, delete_response};
use betterlite_openai::videos::error::{VideoError, get_error_class};
use betterlite_openai::videos::list::{list_request, list_response};
use betterlite_openai::videos::remix::{remix_request, remix_response};
use betterlite_openai::videos::reply::VideoObject;
use betterlite_openai::videos::status::{status_request, status_response};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

use super::input::{
    Body, Call, ContentRequest, Create, CreateResponse, Environment, ListRequest, Reference,
    RemixRequest, Request, Response,
};

type PathRequest = fn(&str, &str) -> Result<(String, VideoOptions), VideoError>;
type VideoReply = fn(&[u8], Option<&str>) -> Result<VideoObject, VideoError>;

pub(super) fn run_call(call: &Call) -> HelperResult {
    match call {
        Call::Create(create) => create_view(create),
        Call::CreateResponse(response) => create_response_view(response),
        Call::ContentRequest(request) => content_request_view(request),
        Call::ContentResponse(body) => {
            let bytes = STANDARD.decode(&body.body_base64).or_value_error()?;
            Ok(json!({"body_base64": STANDARD.encode(content_response(&bytes))}))
        }
        Call::RemixRequest(request) => remix_request_view(request),
        Call::RemixResponse(response) => video_response_view(response, remix_response),
        Call::ListRequest(request) => list_request_view(request),
        Call::ListResponse(response) => list_response_view(response),
        Call::DeleteRequest(request) => path_request_view(request, delete_request),
        Call::DeleteResponse(body) => delete_response_view(body),
        Call::StatusRequest(request) => path_request_view(request, status_request),
        Call::StatusResponse(response) => video_response_view(response, status_response),
        Call::ValidateEnvironment(environment) => environment_view(environment),
        Call::CompleteUrl(call) => Ok(json!({"url": complete_url(call.api_base.as_deref())})),
        Call::ErrorClass(call) => {
            let error = get_error_class(&call.message, call.status, call.headers.clone());
            Err(Box::new(HelperError {
                class: "BaseLLMException".to_owned(),
                status: Some(error.status_code),
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.message,
            }))
        }
    }
}

fn files_view(files: &[MultipartPart<'_>]) -> Vec<Value> {
    files
        .iter()
        .map(|part| {
            let (body_kind, body) = match &part.body {
                MediaBody::Bytes(bytes) => ("bytes", STANDARD.encode(bytes)),
                MediaBody::Text(text) => ("text", (*text).to_owned()),
                MediaBody::Stream { .. } => ("stream", String::new()),
            };
            json!({
                "field": part.field.as_str(),
                "filename": part.filename.as_str(),
                "content_type": part.content_type,
                "body_kind": body_kind,
                "body": body,
            })
        })
        .collect()
}

fn create_view(call: &Create) -> HelperResult {
    let reference = call.reference.as_ref();
    let decoded = match reference {
        Some(Reference::Bytes { data_base64 }) => {
            Some(STANDARD.decode(data_base64).or_value_error()?)
        }
        Some(Reference::Text { .. }) | None => None,
    };
    let media = match (reference, decoded.as_deref()) {
        (Some(Reference::Text { text }), _) => Some(MediaBody::Text(text)),
        (Some(Reference::Bytes { .. }), Some(bytes)) => Some(MediaBody::Bytes(bytes)),
        (Some(Reference::Bytes { .. }), None) | (None, _) => None,
    };
    let form = create_request(
        &call.model,
        &call.prompt,
        &call.api_base,
        &call.options,
        media,
    );
    let files = files_view(&form.files);
    Ok(json!({
        "supported": SUPPORTED_PARAMS,
        "data": form.fields,
        "files": files,
        "api_base": form.api_base,
        "multipart": MULTIPART_FORM_DATA,
    }))
}

fn content_request_view(call: &ContentRequest) -> HelperResult {
    let (url, params) = content_request(&call.video_id, &call.api_base, call.variant.as_deref())
        .or_value_error()?;
    Ok(json!({"url": url, "params": params}))
}

fn remix_request_view(call: &RemixRequest) -> HelperResult {
    let (url, params) = remix_request(
        &call.video_id,
        &call.prompt,
        &call.api_base,
        &VideoOptions::new(),
    )
    .or_value_error()?;
    Ok(json!({"url": url, "params": params}))
}

fn list_request_view(call: &ListRequest) -> HelperResult {
    let (url, params) = list_request(
        &call.api_base,
        call.after.as_deref(),
        call.limit,
        call.order.as_deref(),
        &VideoOptions::new(),
    );
    Ok(json!({"url": url, "params": params}))
}

fn path_request_view(call: &Request, transform: PathRequest) -> HelperResult {
    let (url, params) = transform(&call.video_id, &call.api_base).or_value_error()?;
    Ok(json!({"url": url, "params": params}))
}

fn delete_response_view(body: &Body) -> HelperResult {
    let bytes = STANDARD.decode(&body.body_base64).or_value_error()?;
    let video = delete_response(&bytes).or_value_error()?;
    serde_json::to_value(&video).or_value_error()
}

fn create_response_view(call: &CreateResponse) -> HelperResult {
    let bytes = STANDARD.decode(&call.body_base64).or_value_error()?;
    let video = create_response(&call.model, &bytes, call.provider.as_deref()).or_value_error()?;
    serde_json::to_value(&video).or_value_error()
}

fn video_response_view(call: &Response, transform: VideoReply) -> HelperResult {
    let bytes = STANDARD.decode(&call.body_base64).or_value_error()?;
    let video = transform(&bytes, call.provider.as_deref()).or_value_error()?;
    serde_json::to_value(&video).or_value_error()
}

fn list_response_view(call: &Response) -> HelperResult {
    let bytes = STANDARD.decode(&call.body_base64).or_value_error()?;
    let list = list_response(&bytes, call.provider.as_deref()).or_value_error()?;
    serde_json::to_value(&list).or_value_error()
}

fn environment_view(call: &Environment) -> HelperResult {
    let configured = VideoCredentials {
        api_key: call.api_key.as_deref().map(SecretString::from),
        params_api_key: call.params_api_key.as_deref().map(SecretString::from),
        global_api_key: call.global_api_key.as_deref().map(SecretString::from),
        openai_key: call.openai_key.as_deref().map(SecretString::from),
        environment_api_key: call.environment_api_key.as_deref().map(SecretString::from),
    };
    let given = call
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect();
    let updated = validate_environment(given, &configured);
    let exposed: BTreeMap<&str, &str> = updated
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    Ok(json!({"headers": exposed}))
}
