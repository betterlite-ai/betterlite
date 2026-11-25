use super::convert::fields::{target, text};
use super::convert::options::edit_options;
use super::convert::{
    DEFAULT_IMAGE_NAME, DEFAULT_MASK_NAME, ImageAnswer, ImageCallWire, OPENAI_PROVIDER,
    content_type_for, images,
};
use super::error::{CallFailure, connection_failure, convert_failure, provider_failure};
use super::session::{CallSetup, ImageSession, call_setup, now_seconds};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageFiles};
use betterlite_openai::image_edits::environment::{complete_url, validate_environment};
use betterlite_openai::image_edits::family::select_image_edit_family;
use betterlite_openai::image_edits::file::{ImageEditFile, multipart_parts};
use betterlite_openai::image_edits::request::{
    ImageEditForm, ImageEditRequest, transform_image_edit_request,
};
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
    let (model, provider) =
        target(&call.arguments).map_err(|error| convert_failure(&error, None, None))?;
    if provider != OPENAI_PROVIDER {
        return Err(connection_failure(
            format!("image edit is not supported for {provider}"),
            Some(model),
            Some(provider),
        ));
    }
    let setup = call_setup(session, &call.arguments, &call.settings, &model, &provider)?;
    if setup.credentials.api_key.is_none() {
        return Err(provider_failure(
            String::from(MISSING_API_KEY),
            None,
            &model,
            &provider,
            &session.redactor,
        ));
    }
    let outbound = outbound(session, &call, &files, &model, setup)?;
    let reply = round(session, outbound, &model, &provider).await?;
    Ok(ImageAnswer::new(reply, None))
}

fn outbound(
    session: &ImageSession,
    call: &ImageCallWire,
    files: &[(String, CallFile)],
    model: &str,
    setup: CallSetup,
) -> Result<OutboundRequest, CallFailure> {
    let provider = OPENAI_PROVIDER;
    let mut form = form(call, files, model, provider, &session.redactor)?;
    for (_field, file) in &mut form.files {
        let name = file
            .filename
            .clone()
            .unwrap_or_else(|| DEFAULT_IMAGE_NAME.to_owned());
        file.content_type = Some(content_type_for(&name));
    }
    let url = complete_url(&setup.credentials.api_base).map_err(|error| {
        provider_failure(
            error.to_string(),
            error.status(),
            model,
            provider,
            &session.redactor,
        )
    })?;
    let headers = validate_environment(&setup.headers, &setup.credentials).map_err(|error| {
        provider_failure(
            error.to_string(),
            error.status(),
            model,
            provider,
            &session.redactor,
        )
    })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Multipart {
            boundary: Uuid::new_v4().to_string(),
            parts: parts(&form.data, &form.files),
        },
        deadline: setup.timeout,
    })
}

pub(crate) fn form(
    call: &ImageCallWire,
    files: &[(String, CallFile)],
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> Result<ImageEditForm, CallFailure> {
    let options = edit_options(
        &call.arguments,
        model,
        call.settings.drop_params.unwrap_or(false),
    )
    .map_err(|error| convert_failure(&error, Some(model.to_owned()), Some(provider.to_owned())))?;
    let prompt = text(&call.arguments, "prompt").map_err(|error| {
        convert_failure(&error, Some(model.to_owned()), Some(provider.to_owned()))
    })?;
    let mask = images(files, "mask", DEFAULT_MASK_NAME)
        .pop()
        .map(|file| vec![Some(edit_file(&file))]);
    let request = ImageEditRequest {
        model: model.to_owned(),
        prompt,
        image: Some(
            images(files, "image", DEFAULT_IMAGE_NAME)
                .into_iter()
                .map(|file| Some(edit_file(&file)))
                .collect(),
        ),
        mask,
        options,
    };
    transform_image_edit_request(select_image_edit_family(model), &request).map_err(|error| {
        provider_failure(error.to_string(), error.status(), model, provider, redactor)
    })
}

async fn round(
    session: &ImageSession,
    outbound: OutboundRequest,
    model: &str,
    provider: &str,
) -> Result<ImageGenerationReply, CallFailure> {
    let response = session.client.generate(outbound).await.map_err(|error| {
        provider_failure(error.to_string(), None, model, provider, &session.redactor)
    })?;
    reply(&response, model, provider, &session.redactor)
}

pub(crate) fn edit_file(file: &ImageFile) -> ImageEditFile {
    ImageEditFile {
        content: file.content.clone(),
        name: file.filename.clone(),
    }
}

pub(crate) fn parts(data: &RequestMap, files: &ImageFiles) -> Vec<MultipartPart> {
    let mut parts = Vec::new();
    for (name, value) in data {
        let text = match value {
            Json::Text(text) => text.clone(),
            Json::Null => String::new(),
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                serde_json::to_string(value).unwrap_or_default()
            }
        };
        parts.push(MultipartPart {
            name: name.clone(),
            file_name: None,
            content_type: None,
            data: text.into_bytes(),
        });
    }
    parts.extend(multipart_parts(files));
    parts
}

pub(crate) fn reply(
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
    transform_image_edit_response(response.status, &response.body, now_seconds()).map_err(|error| {
        provider_failure(error.to_string(), error.status(), model, provider, redactor)
    })
}

#[derive(Deserialize)]
struct StatusBody {
    error: Option<StatusError>,
}

#[derive(Deserialize)]
struct StatusError {
    message: Option<String>,
}

pub(crate) fn status_message(body: &[u8]) -> String {
    let parsed = serde_json::from_slice::<StatusBody>(body);
    match parsed
        .ok()
        .and_then(|found| found.error)
        .and_then(|error| error.message)
    {
        Some(message) => message,
        None => str::from_utf8(body).unwrap_or_default().to_owned(),
    }
}

mod tests;
