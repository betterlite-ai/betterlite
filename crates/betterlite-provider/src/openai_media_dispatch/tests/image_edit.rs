use betterlite_openai::image_edits::error::ImageEditError;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::image_edits::request::ImageEditRequest;

use super::fixture::{Fixture, Recorder, media_error, media_result, options};
use crate::openai_media_dispatch::call::{ImageEditCall, OpenAiMediaOperation};
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{
    MediaMethod, MediaPartBody, MediaPayload, OpenAiMediaPrepared, OpenAiMediaRequest,
    OpenAiMediaResult,
};
use crate::operation::{OperationRequest, OperationResult};

const API_BASE: &str = "https://images.test/v1";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nimage";

fn file(name: Option<&str>) -> ImageEditFile {
    ImageEditFile {
        content: PNG.to_vec(),
        name: name.map(str::to_owned),
    }
}

fn request(
    model: &str,
    images: Vec<Option<ImageEditFile>>,
    mask: Option<Vec<Option<ImageEditFile>>>,
) -> ImageEditRequest {
    ImageEditRequest {
        model: String::from(model),
        prompt: Some(String::from("add a hat")),
        image: Some(images),
        mask,
        options: options(r#"{"size":"1024x1024"}"#),
    }
}

async fn run(
    model: &str,
    images: Vec<Option<ImageEditFile>>,
    mask: Option<Vec<Option<ImageEditFile>>>,
    fixture: &Fixture,
) -> Result<OperationResult, OpenAiMediaError> {
    let call = ImageEditCall {
        api_base: String::from(API_BASE),
        request: request(model, images, mask),
    };
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::ImageEdit(Box::new(call)),
        fixture,
        &mut logger,
    )
    .await
}

pub(super) async fn check() {
    gpt_images().await;
    dalle_image().await;
    multiple_images().await;
}

async fn gpt_images() {
    let fixture = Fixture::new(200, b"");
    let result = media_result(
        run(
            "gpt-image-1",
            vec![Some(file(None)), Some(file(Some("second.png")))],
            Some(vec![Some(file(None))]),
            &fixture,
        )
        .await,
    );
    let OpenAiMediaResult::ImageEdit(prepared) = result else {
        panic!("the edit stays its own prepared operation");
    };
    assert_eq!(prepared.method, MediaMethod::Post);
    assert_eq!(prepared.url, "https://images.test/v1/images/edits");
    assert!(prepared.multipart);
    gpt_form(&prepared);
}

fn gpt_form(prepared: &OpenAiMediaPrepared) {
    let MediaPayload::ImageData(data) = &prepared.body else {
        panic!("the edit sends its form data");
    };
    assert_eq!(
        data,
        &options(r#"{"model":"gpt-image-1","size":"1024x1024","prompt":"add a hat"}"#)
    );
    let fields: Vec<&str> = prepared
        .parts
        .iter()
        .map(|part| part.field.as_str())
        .collect();
    assert_eq!(fields, ["image[]", "image[]", "mask"]);
    let names: Vec<Option<&str>> = prepared
        .parts
        .iter()
        .map(|part| part.filename.as_deref())
        .collect();
    assert_eq!(
        names,
        [Some("image.png"), Some("second.png"), Some("mask.png")]
    );
    assert!(
        prepared
            .parts
            .iter()
            .all(|part| part.content_type.as_deref() == Some("image/png"))
    );
    assert!(
        prepared
            .parts
            .iter()
            .all(|part| part.body == MediaPartBody::Bytes(PNG.to_vec()))
    );
}

async fn dalle_image() {
    let fixture = Fixture::new(200, b"");
    let result = media_result(
        run(
            "dall-e-2",
            vec![Some(file(None))],
            Some(vec![Some(file(None))]),
            &fixture,
        )
        .await,
    );
    let OpenAiMediaResult::ImageEdit(prepared) = result else {
        panic!("the edit stays its own prepared operation");
    };
    let fields: Vec<&str> = prepared
        .parts
        .iter()
        .map(|part| part.field.as_str())
        .collect();
    assert_eq!(fields, ["image", "mask"]);
    assert_eq!(prepared.url, "https://images.test/v1/images/edits");
}

async fn multiple_images() {
    let fixture = Fixture::new(200, b"");
    let error = media_error(
        run(
            "dall-e-2",
            vec![Some(file(None)), Some(file(None))],
            None,
            &fixture,
        )
        .await,
    );
    let OpenAiMediaError::ImageEdit(image_edit) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    assert_eq!(image_edit.status(), Some(400));
    assert!(matches!(image_edit, ImageEditError::MultipleImages));
}
