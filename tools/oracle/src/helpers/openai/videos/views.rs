use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::openai_video_identifiers::multipart::{MediaBody, MultipartPart};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::{
    MULTIPART_FORM_DATA, SUPPORTED_PARAMS, VideoCredentials, complete_url, validate_environment,
};
use betterlite_openai::videos::content::content_request;
use betterlite_openai::videos::create::{create_request, create_response};
use betterlite_openai::videos::error::{VideoError, get_error_class};
use betterlite_openai::videos::list::{list_request, list_response};
use betterlite_openai::videos::remix::{remix_request, remix_response};
use betterlite_openai::videos::reply::{VideoObject, decode_video};
use betterlite_openai::videos::segment::path_url;
use betterlite_openai::videos::status::status_response;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

use super::input::{
    AuthCall, Body, Call, Create, CreateResponse, ErrorCall, ListRequest, Reference, Response,
};

type VideoReply = fn(&[u8], Option<&str>) -> Result<VideoObject, VideoError>;

pub(super) fn run_call(call: &Call) -> HelperResult {
    match call {
        Call::Create(create) => create_view(create),
        Call::CreateResponse(response) => create_response_view(response),
        Call::ContentRequest(request) => request_view(content_request(
            &request.video_id,
            &request.api_base,
            request.variant.as_deref(),
        )),
        Call::ContentResponse(body) => {
            let bytes = STANDARD.decode(&body.body_base64).or_value_error()?;
            Ok(json!({"body_base64": STANDARD.encode(bytes)}))
        }
        Call::RemixRequest(request) => request_view(remix_request(
            &request.video_id,
            &request.prompt,
            &request.api_base,
            &VideoOptions::new(),
        )),
        Call::RemixResponse(response) => video_response_view(response, remix_response),
        Call::ListRequest(request) => list_request_view(request),
        Call::ListResponse(response) => list_response_view(response),
        Call::DeleteRequest(request) | Call::StatusRequest(request) => request_view(
            path_url(&request.video_id, &request.api_base, "")
                .map(|path| (path, VideoOptions::new())),
        ),
        Call::DeleteResponse(body) => delete_response_view(body),
        Call::StatusResponse(response) => video_response_view(response, status_response),
        Call::ValidateEnvironment(environment) => Ok(environment_view(environment)),
        Call::CompleteUrl(complete) => {
            Ok(json!({"url": complete_url(complete.api_base.as_deref())}))
        }
        Call::ErrorClass(failure) => error_class_view(failure),
    }
}

fn error_class_view(call: &ErrorCall) -> HelperResult {
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

fn request_view(request: Result<(String, VideoOptions), VideoError>) -> HelperResult {
    request
        .or_value_error()
        .map(|(url, params)| json!({"url": url, "params": params}))
}

fn list_request_view(call: &ListRequest) -> HelperResult {
    let listed = list_request(
        &call.api_base,
        call.after.as_deref(),
        call.limit,
        call.order.as_deref(),
        &VideoOptions::new(),
    );
    request_view(Ok(listed))
}

fn delete_response_view(body: &Body) -> HelperResult {
    let bytes = STANDARD.decode(&body.body_base64).or_value_error()?;
    let video = decode_video(&bytes).or_value_error()?;
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

fn environment_view(call: &AuthCall) -> Value {
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
    json!({"headers": exposed})
}
