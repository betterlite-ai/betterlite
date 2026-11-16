use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::api::image_contracts::records::{
    ImageHttpResponse, ImageParams, ImageResponse, ImageUsage,
};
use betterlite_openai::openrouter_chat::error::OpenRouterError;
use betterlite_openai::openrouter_image::edit::{
    SourceImage, complete_url as edit_url, request_body as edit_request_body,
    transform_response as edit_response, validate_environment as edit_headers,
};
use std::io::Cursor;

// ast-grep-ignore: no-long-names
use betterlite_openai::openrouter_image::edit::get_openrouter_image_edit_config as edit_config;

use super::input::{GenerationParamsCase, ImageCase, Input, ReplyCase, RequestCase};

// ast-grep-ignore: no-long-names
use betterlite_openai::openrouter_image::generation::get_openrouter_image_generation_config as generation_config;
use betterlite_openai::openrouter_image::generation::{
    complete_url as generation_url, request_body as generation_request_body,
    transform_response as generation_response, validate_environment as generation_headers,
};
use betterlite_openai::openrouter_image::shared::{ImageConfigError, ImageHiddenParams};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::SecretString;
use serde_json::{Value, json};
pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::GenerationParams { cases } => generation_params(cases),
        Input::GenerationRequest { cases } => Ok(generation_request(cases)),
        Input::GenerationReply { cases } => replies(cases, generation_reply),
        Input::EditParams { cases } => edit_params(cases),
        Input::EditRequest { cases } => edit_requests(cases),
        Input::EditReply { cases } => replies(cases, edit_reply),
    }
}
fn keys(case: &RequestCase) -> [Option<SecretString>; 3] {
    let environment = case.environment.get("OPENROUTER_API_KEY").cloned();
    [
        case.api_key.clone(),
        case.global_api_key.clone(),
        environment,
    ]
    .map(|key| key.map(SecretString::from))
}
fn generation_params(cases: &[GenerationParamsCase]) -> HelperResult {
    let mut answers = Vec::with_capacity(cases.len());
    for case in cases {
        let config = generation_config(&case.model);
        let mapped = config
            .map_params(
                &case.non_default_params,
                case.optional_params.clone(),
                case.drop_params,
            )
            .or_value_error()?;
        answers.push(json!({
            "supported": config.supported_params,
            "params": mapped,
        }));
    }
    Ok(Value::Array(answers))
}
fn generation_request(cases: &[RequestCase]) -> Value {
    let mut answers = Vec::with_capacity(cases.len());
    for case in cases {
        let [api_key, global_api_key, environment_key] = keys(case);
        let headers = generation_headers(
            case.headers.clone(),
            api_key.as_ref(),
            global_api_key.as_ref(),
            environment_key.as_ref(),
        );
        answers.push(json!({
            "url": generation_url(case.api_base.as_deref()),
            "headers": headers,
            "request": generation_request_body(
                &case.model,
                case.prompt.as_deref().unwrap_or_default(),
                &case.optional_params,
            ),
        }));
    }
    Value::Array(answers)
}
fn edit_params(cases: &[ImageParams]) -> HelperResult {
    let config = edit_config("");
    let answers = cases
        .iter()
        .map(|case| {
            let params = config.map_params(case)?;
            Ok(json!({"supported": config.supported_params, "params": params}))
        })
        .collect::<Result<Vec<_>, ImageConfigError>>()
        .or_value_error()?;
    Ok(Value::Array(answers))
}
fn edit_requests(cases: &[RequestCase]) -> HelperResult {
    let mut answers = Vec::with_capacity(cases.len());
    for case in cases {
        let [api_key, global_api_key, environment_key] = keys(case);
        let headers = edit_headers(
            case.headers.clone(),
            api_key.as_ref(),
            global_api_key.as_ref(),
            environment_key.as_ref(),
        )
        .or_value_error()?;
        let url = edit_url(
            case.api_base.as_deref(),
            case.environment
                .get("OPENROUTER_API_BASE")
                .map(String::as_str),
        );
        let payload = case
            .image
            .as_ref()
            .map(|image| match image {
                ImageCase::Bytes { data } | ImageCase::Reader { data, .. } => STANDARD.decode(data),
            })
            .transpose()
            .or_value_error()?;
        let (request, files, position) =
            image_request(case, payload.as_deref()).or_value_error()?;
        answers.push(json!({
            "url": url,
            "headers": headers,
            "request": request,
            "files": files,
            "position": position,
        }));
    }
    Ok(Value::Array(answers))
}
fn image_request(
    case: &RequestCase,
    payload: Option<&[u8]>,
) -> Result<(ImageParams, Vec<String>, Option<u64>), ImageConfigError> {
    let mut reader = Cursor::new(payload.unwrap_or_default());
    if let Some(ImageCase::Reader { position, .. }) = &case.image {
        reader.set_position(*position);
    }
    let mut images = [match &case.image {
        Some(ImageCase::Bytes { .. }) => Some(SourceImage::Bytes(payload.unwrap_or_default())),
        Some(ImageCase::Reader { .. }) => Some(SourceImage::Reader(&mut reader)),
        None => None,
    }];
    let (request, files) = edit_request_body(
        &case.model,
        case.prompt.as_deref(),
        &mut images,
        &case.optional_params,
    )?;
    let position = match &case.image {
        Some(ImageCase::Reader { .. }) => Some(reader.position()),
        Some(ImageCase::Bytes { .. }) | None => None,
    };
    let names = files.into_iter().map(|(name, _)| name).collect();
    Ok((request, names, position))
}
fn replies(cases: &[ReplyCase], answer: fn(&ReplyCase) -> HelperResult) -> HelperResult {
    let mut answers = Vec::with_capacity(cases.len());
    for case in cases {
        answers.push(answer(case)?);
    }
    Ok(Value::Array(answers))
}
fn generation_reply(case: &ReplyCase) -> HelperResult {
    let raw = raw_response(case);
    let model_response = ImageResponse {
        created: case.created,
        data: Some(Vec::new()),
        usage: Some(ImageUsage::default()),
    };
    let (response, hidden) = generation_response(
        &raw,
        model_response,
        ImageHiddenParams::default(),
        &case.model,
    )
    .map_err(reply_failure)?;
    Ok(json!({
        "created": response.created,
        "data": response.data.unwrap_or_default(),
        "usage": response.usage,
        "hidden": hidden,
    }))
}
fn edit_reply(case: &ReplyCase) -> HelperResult {
    edit_response(&raw_response(case), case.created, &case.model)
        .map_err(reply_failure)
        .map(|(response, hidden)| {
            json!({
                "data": response.data.unwrap_or_default(),
                "usage": response.usage,
                "hidden": hidden,
            })
        })
}
fn raw_response(case: &ReplyCase) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: case.status,
        headers: case.headers.clone(),
        body: case.body.as_bytes().to_vec(),
    }
}
fn reply_failure(error: OpenRouterError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("OpenRouterError"),
        status: u16::try_from(error.status_code).ok(),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.message,
    })
}
