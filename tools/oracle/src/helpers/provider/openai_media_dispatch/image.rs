use std::time::Duration;

use betterlite_openai::api::image_contracts::records::ImageFile;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditRequest;
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_variations::{ImageVariationParams, ImageVariationRequest};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::OpenAiMediaResult;
use betterlite_provider::openai_media_dispatch::call::{ImageEditCall, OpenAiMediaOperation};
use betterlite_provider::openai_media_dispatch::error::OpenAiMediaError;
use betterlite_provider::operation::OperationResult;
use secrecy::SecretString;
use serde_json::Value;

use super::fixtures::{self, Fixture};
use super::views::{image_reply, parts_value, payload_value};
use super::{
    ImageEditCase, ImageFileCase, ImageGenerationCase, ImageVariationCase, LeafResult,
    OTHER_OUTCOME, failure, leaf_failure, run,
};

const NOW: i64 = 1_700_000_000;
const TIMEOUT: Duration = Duration::from_secs(60);

// The two image leaves take different calls and read different requests, so one case names which it runs.
#[derive(Clone, Copy)]
pub(super) enum ImageLeaf<'case> {
    Generation(&'case ImageGenerationCase),
    Variation(&'case ImageVariationCase),
}

pub(super) async fn image_leaf(leaf: ImageLeaf<'_>) -> HelperResult {
    let (status, reply) = match leaf {
        ImageLeaf::Generation(case) => (case.status, &case.reply),
        ImageLeaf::Variation(case) => (case.status, &case.reply),
    };
    let reply_body = serde_json::to_vec(reply).map_err(|error| failure("Serialize", error))?;
    let fixture = Fixture::new(status, &reply_body);
    let result = run(&fixture, leaf_operation(leaf)).await;
    let sent = fixtures::only_request(&fixture)?;
    let request = match leaf {
        ImageLeaf::Generation(_) => serde_json::json!({
            "method": sent.method.as_str(),
            "url": sent.url.as_str(),
            "body": fixtures::json_body(&sent)?,
            "authorization": fixtures::header(&sent, "authorization"),
        }),
        ImageLeaf::Variation(_) => serde_json::json!({
            "url": sent.url.as_str(),
            "authorization": fixtures::header(&sent, "authorization"),
            "parts": fixtures::multipart_view(&sent)?,
        }),
    };
    image_answer(&request, result)
}

fn leaf_operation(leaf: ImageLeaf<'_>) -> OpenAiMediaOperation {
    match leaf {
        ImageLeaf::Generation(case) => {
            OpenAiMediaOperation::ImageGeneration(Box::new(ImageGenerationCall {
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
            }))
        }
        ImageLeaf::Variation(case) => {
            OpenAiMediaOperation::ImageVariation(Box::new(variation_request(case)))
        }
    }
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
    let fixture = Fixture::new(200, b"");
    let result = run(&fixture, OpenAiMediaOperation::ImageEdit(Box::new(call))).await;
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

// Both image leaves answer with the same envelope, so one match names the reply each outcome carries.
fn image_answer(request: &Value, result: LeafResult) -> HelperResult {
    let (reply, raised) = match result {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::ImageGeneration(outcome))) => {
            (image_reply(&outcome.reply), Value::Null)
        }
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::ImageVariation(outcome))) => {
            (image_reply(&outcome.reply), Value::Null)
        }
        Ok(_) => return Err(failure("Variant", OTHER_OUTCOME)),
        Err(OpenAiMediaError::ImageGeneration(error)) => (
            Value::Null,
            serde_json::json!({"mapped": error.mapped(), "status": error.status()}),
        ),
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
