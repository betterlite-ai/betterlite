use betterlite_openai::api::image_contracts::records::ImageFile;
use betterlite_openai::image_variations::error::OpenAiImageVariationError;
use betterlite_openai::image_variations::{ImageVariationParams, ImageVariationRequest};
use http::Method;
use secrecy::SecretString;

use super::fixture::{Fixture, Recorder, boundary, media_error, media_result, options, parts};
use crate::openai_media_dispatch::call::OpenAiMediaOperation;
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{OpenAiMediaRequest, OpenAiMediaResult};
use crate::operation::{OperationRequest, OperationResult};

const API_BASE: &str = "https://images.test/v1";
const NOW: i64 = 1_700_000_003;
const PNG: &[u8] = b"\x89PNG\r\n\x1a\ncat";
const REPLY: &[u8] = br#"{"created":1700000003,"data":[{"url":"https://cdn.test/cat.png"}]}"#;

fn request(api_key: Option<&str>) -> ImageVariationRequest {
    ImageVariationRequest {
        model: Some(String::from("dall-e-2")),
        image: ImageFile {
            filename: Some(String::from("cat.png")),
            content: PNG.to_vec(),
            content_type: Some(String::from("image/png")),
        },
        optional_params: options(r#"{"n":2,"size":"256x256"}"#),
        headers: None,
        api_key: api_key.map(SecretString::from),
        api_base: Some(String::from(API_BASE)),
        organization: None,
        timeout: None,
        params: ImageVariationParams::default(),
        client: None,
        shared_session: None,
        config: None,
        boundary: String::from("boundary-1"),
        now_seconds: NOW,
    }
}

async fn run(
    request: ImageVariationRequest,
    fixture: &Fixture,
) -> Result<OperationResult, OpenAiMediaError> {
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::ImageVariation(Box::new(request)),
        fixture,
        &mut logger,
    )
    .await
}

pub(super) async fn check() {
    variation().await;
    missing_key().await;
}

async fn variation() {
    let fixture = Fixture::new(200, REPLY);
    let result = media_result(run(request(Some("sk-variation")), &fixture).await);
    let OpenAiMediaResult::ImageVariation(outcome) = result else {
        panic!("the variation stays its own operation");
    };
    let sent = fixture
        .take_request()
        .expect("one dispatch sends one request");
    assert_eq!(sent.method, Method::POST);
    assert_eq!(
        sent.url.as_str(),
        "https://images.test/v1/images/variations"
    );
    assert!(!sent.url.as_str().ends_with("/images/edits"));
    assert_eq!(boundary(&sent), "boundary-1");
    let form = parts(&sent);
    let names: Vec<&str> = form.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(names, ["n", "size", "image"]);
    assert!(form.len() > 2);
    assert_eq!(form[0].data, b"2".to_vec());
    assert_eq!(form[1].data, b"256x256".to_vec());
    assert_eq!(form[2].file_name.as_deref(), Some("cat.png"));
    assert_eq!(form[2].content_type.as_deref(), Some("image/png"));
    assert_eq!(form[2].data, PNG.to_vec());
    assert_eq!(
        outcome.reply.data[0].url.as_deref(),
        Some("https://cdn.test/cat.png")
    );
}

async fn missing_key() {
    let fixture = Fixture::new(200, REPLY);
    let error = media_error(run(request(None), &fixture).await);
    let OpenAiMediaError::ImageVariation(variation) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    let OpenAiImageVariationError {
        status_code,
        message,
        ..
    } = variation;
    assert_eq!(status_code, 500);
    assert_eq!(message, "no API key is set for the image variation call");
}
