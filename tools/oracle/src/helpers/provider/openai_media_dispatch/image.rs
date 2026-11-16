use std::time::Duration;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::api::image_contracts::records::ImageFile;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditRequest;
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_variations::{ImageVariationParams, ImageVariationRequest};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::call::{ImageEditCall, OpenAiMediaOperation};
use betterlite_provider::openai_media_dispatch::error::OpenAiMediaError;
use betterlite_provider::openai_media_dispatch::execute::dispatch;
use betterlite_provider::openai_media_dispatch::{OpenAiMediaRequest, OpenAiMediaResult};
use betterlite_provider::operation::{OperationRequest, OperationResult};
use secrecy::SecretString;
use serde_json::Value;

use super::fixtures::{self, Fixture, Recorder};
use super::views::{image_reply, parts_value, payload_value, reply_bytes};
use super::{
    ImageEditCase, ImageFileCase, ImageGenerationCase, ImageVariationCase, OTHER_OUTCOME, failure,
    leaf_failure,
};

const NOW: i64 = 1_700_000_000;
const TIMEOUT: Duration = Duration::from_secs(60);

pub(super) async fn image_generation(case: &ImageGenerationCase) -> HelperResult {
    let call = ImageGenerationCall {
        model: case.model.clone(),
        prompt: case.prompt.clone(),
        optional_params: case.options.clone(),
        api_key: Some(SecretString::from("sk-media")),
        api_base: Some(case.api_base.clone()),
        organization: None,
        headers: None,
        timeout: TIMEOUT,
        delegate_async: false,
        now_seconds: NOW,
    };
    let fixture = Fixture::new(case.status, reply_bytes(&case.reply)?);
    let mut logger = Recorder;
    let result = dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::ImageGeneration(Box::new(call)),
        &fixture,
        &mut logger,
    )
    .await;
    let request = request_view(&fixture.only_request()?)?;
    let (reply, raised) = match result {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::ImageGeneration(outcome))) => {
            (image_reply(&outcome.reply), Value::Null)
        }
        Ok(_) => return Err(failure("Variant", OTHER_OUTCOME)),
        Err(OpenAiMediaError::ImageGeneration(error)) => (
            Value::Null,
            serde_json::json!({"mapped": error.mapped(), "status": error.status()}),
        ),
        Err(error) => return Err(leaf_failure(&error)),
    };
    Ok(serde_json::json!({"request": request, "reply": reply, "error": raised}))
}

pub(super) async fn image_edit(case: &ImageEditCase) -> HelperResult {
    let call = ImageEditCall {
        api_base: case.api_base.clone(),
        request: ImageEditRequest {
            model: case.model.clone(),
            prompt: case.prompt.clone(),
            image: Some(files(&case.image)),
            mask: case.mask.as_ref().map(|mask| files(mask)),
            options: case.options.clone(),
        },
    };
    let fixture = Fixture::new(200, Vec::new());
    let mut logger = Recorder;
    let result = dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::ImageEdit(Box::new(call)),
        &fixture,
        &mut logger,
    )
    .await;
    if fixture.request_count() != 0 {
        return Err(failure("Fixture", "the image edit sends no request"));
    }
    match result {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::ImageEdit(prepared))) => {
            Ok(serde_json::json!({
                "url": prepared.url,
                "data": payload_value(&prepared.body)?,
                "parts": parts_value(&prepared.parts)?,
            }))
        }
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(&error)),
    }
}

pub(super) async fn image_variation(case: &ImageVariationCase) -> HelperResult {
    let operation = variation_request(case);
    let fixture = Fixture::new(case.status, reply_bytes(&case.reply)?);
    let mut logger = Recorder;
    let result = dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::ImageVariation(Box::new(operation)),
        &fixture,
        &mut logger,
    )
    .await;
    let sent = fixture.only_request()?;
    let request = serde_json::json!({
        "url": sent.url.as_str(),
        "authorization": fixtures::header(&sent, "authorization"),
        "parts": fixtures::multipart_view(&sent)?,
    });
    let (reply, raised) = match result {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::ImageVariation(outcome))) => {
            (image_reply(&outcome.reply), Value::Null)
        }
        Ok(_) => return Err(failure("Variant", OTHER_OUTCOME)),
        Err(OpenAiMediaError::ImageVariation(error)) => (
            Value::Null,
            serde_json::json!({"mapped": true, "status": error.status_code}),
        ),
        Err(error) => return Err(leaf_failure(&error)),
    };
    Ok(serde_json::json!({"request": request, "reply": reply, "error": raised}))
}

fn variation_request(case: &ImageVariationCase) -> ImageVariationRequest {
    ImageVariationRequest {
        model: Some(case.model.clone()),
        image: ImageFile {
            filename: case.image_filename.clone(),
            content: case.image_content.clone().into_bytes(),
            content_type: case.image_content_type.clone(),
        },
        optional_params: case.options.clone(),
        headers: None,
        api_key: Some(SecretString::from("sk-variation")),
        api_base: Some(case.api_base.clone()),
        organization: None,
        timeout: Some(TIMEOUT),
        params: ImageVariationParams::default(),
        client: None,
        shared_session: None,
        config: None,
        boundary: String::from("media-boundary"),
        now_seconds: NOW,
    }
}

fn files(case_files: &[ImageFileCase]) -> Vec<Option<ImageEditFile>> {
    case_files
        .iter()
        .map(|file| {
            Some(ImageEditFile {
                content: file.content.clone().into_bytes(),
                name: file.name.clone(),
            })
        })
        .collect()
}

fn request_view(request: &OutboundRequest) -> Result<Value, Box<HelperError>> {
    Ok(serde_json::json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "body": fixtures::json_body(request)?,
        "authorization": fixtures::header(request, "authorization"),
    }))
}
