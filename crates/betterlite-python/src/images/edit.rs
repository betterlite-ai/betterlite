use super::convert::fields::{headers, resolve_target, text, timeout};
use super::convert::options::edit_options;
use super::convert::{
    DEFAULT_GENERATION_MODEL, DEFAULT_TIMEOUT, ImageAnswer, ImageCallWire, OPENAI_PROVIDER,
    content_type_for, images, mask_file,
};
use super::error::{CallFailure, connection_failure, convert_failure, provider_failure};
use super::session::{ImageSession, explicit_credentials, now_seconds};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageFiles};
use betterlite_openai::image_edits::environment::{complete_url, validate_environment};
use betterlite_openai::image_edits::error::ImageEditError;
use betterlite_openai::image_edits::family::select_image_edit_family;
use betterlite_openai::image_edits::file::{ImageEditFile, multipart_parts};
use betterlite_openai::image_edits::request::{ImageEditRequest, transform_image_edit_request};
use betterlite_openai::image_edits::response::transform_image_edit_response;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_openai::image_generation::transport::ImageGenerationTransport as _;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_files::CallFile;
use betterlite_sdk::call_setup::RequestMap;
use http::Method;
use serde::Deserialize;
use uuid::Uuid;

const MISSING_API_KEY: &str = "The api_key client option must be set either by passing api_key to the client or by setting the OPENAI_API_KEY environment variable";

pub(crate) async fn edit(
    session: &ImageSession,
    call: ImageCallWire,
    files: Vec<(String, CallFile)>,
) -> Result<ImageAnswer, CallFailure> {
    let requested = text(&call.arguments, "model")
        .map_err(|error| convert_failure(&error, None, None))?
        .unwrap_or_else(|| DEFAULT_GENERATION_MODEL.to_owned());
    let custom = text(&call.arguments, "custom_llm_provider")
        .map_err(|error| convert_failure(&error, Some(requested.clone()), None))?;
    let (model, provider) = resolve_target(&requested, custom.as_deref())
        .map_err(|error| convert_failure(&error, Some(requested.clone()), None))?;
    if provider != OPENAI_PROVIDER {
        return Err(connection_failure(
            format!("image edit is not supported for {provider}"),
            Some(model),
            Some(provider),
        ));
    }
    let options = edit_options(&call.arguments, &model, session.drop_params(&call.settings))
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let prompt = text(&call.arguments, "prompt")
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let mask = mask_file(&files, &call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let headers = headers(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let timeout = timeout(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?
        .unwrap_or(DEFAULT_TIMEOUT);
    let explicit = explicit_credentials(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let credentials = session.credentials(&call.settings, explicit);
    if credentials.api_key.is_none() {
        return Err(provider_failure(
            String::from(MISSING_API_KEY),
            None,
            &model,
            &provider,
            &session.redactor,
        ));
    };
    let request = ImageEditRequest {
        model: model.clone(),
        prompt,
        image: Some(
            images(&files, "image")
                .into_iter()
                .map(|file| Some(edit_file(&file)))
                .collect(),
        ),
        mask: mask.map(|file| vec![Some(edit_file(&file))]),
        options,
    };
    let family = select_image_edit_family(&model);
    let mut form = transform_image_edit_request(family, &request)
        .map_err(|error| edit_failure(&error, &model, &provider, &session.redactor))?;
    for (_field, file) in &mut form.files {
        let name = file
            .filename
            .clone()
            .unwrap_or_else(|| String::from("image.png"));
        file.content_type = Some(content_type_for(&name));
    }
    let url = complete_url(&credentials.api_base)
        .map_err(|error| edit_failure(&error, &model, &provider, &session.redactor))?;
    let headers = validate_environment(&headers, &credentials)
        .map_err(|error| edit_failure(&error, &model, &provider, &session.redactor))?;
    let boundary = Uuid::new_v4().to_string();
    let outbound = OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Multipart {
            boundary,
            parts: parts(&form.data, &form.files),
        },
        deadline: timeout,
    };
    let response = session.client.generate(outbound).await.map_err(|error| {
        provider_failure(
            error.to_string(),
            None,
            &model,
            &provider,
            &session.redactor,
        )
    })?;
    let reply = reply(&response, &model, &provider, &session.redactor)?;
    Ok(ImageAnswer::new(reply, None))
}

fn edit_file(file: &ImageFile) -> ImageEditFile {
    ImageEditFile {
        content: file.content.clone(),
        name: file.filename.clone(),
    }
}

fn parts(data: &RequestMap, files: &ImageFiles) -> Vec<MultipartPart> {
    let mut parts = Vec::new();
    for (name, value) in data {
        parts.push(MultipartPart {
            name: name.clone(),
            file_name: None,
            content_type: None,
            data: field_text(value).into_bytes(),
        });
    }
    parts.extend(multipart_parts(files));
    parts
}

fn field_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null => String::new(),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

fn reply(
    response: &UpstreamResponse,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> Result<ImageGenerationReply, CallFailure> {
    if !(200..=299).contains(&response.status) {
        return Err(provider_failure(
            status_message(&response.body),
            Some(response.status),
            model,
            provider,
            redactor,
        ));
    }
    transform_image_edit_response(response.status, &response.body, now_seconds())
        .map_err(|error| edit_failure(&error, model, provider, redactor))
}

fn edit_failure(
    error: &ImageEditError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallFailure {
    provider_failure(error.to_string(), error.status(), model, provider, redactor)
}

#[derive(Deserialize)]
struct StatusBody {
    error: Option<StatusError>,
}

#[derive(Deserialize)]
struct StatusError {
    message: Option<String>,
}

fn status_message(body: &[u8]) -> String {
    let parsed = serde_json::from_slice::<StatusBody>(body);
    match parsed
        .ok()
        .and_then(|found| found.error)
        .and_then(|error| error.message)
    {
        Some(message) => message,
        None => String::from_utf8_lossy(body).into_owned(),
    }
}
