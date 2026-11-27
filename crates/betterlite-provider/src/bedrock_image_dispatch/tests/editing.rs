use std::time::Duration;

use betterlite_bedrock::image_call::transport::ClientKind;
use betterlite_bedrock::image_edits::plan::EditImage;

use super::edit_fixture::{EditFixture, EditSetup};
use super::fixture::{Run, answer, image_params};
use crate::bedrock_image_dispatch::error::BedrockImageError;
use crate::bedrock_image_dispatch::test_support::NOW_SECONDS;

const CANVAS_MODEL: &str = "amazon.nova-canvas-v1:0";
const STABILITY_MODEL: &str = "stability.stable-image-inpaint-v1:0";
const CANVAS_REPLY: &str = r#"{"images": ["b3V0", "b3V0Mg=="]}"#;
const REGION: &str = r#"{"aws_region_name": "us-east-1"}"#;

pub(super) async fn canvas_edit() {
    canvas_variation().await;
    let inpainting = edited_image(EditCase {
        model: CANVAS_MODEL,
        images: &["aW1n"],
        optional_params: r#"{"aws_region_name": "us-east-1", "mask": "bWFzaw=="}"#,
        reply: r#"{"images": ["b3V0"]}"#,
        expected: r#"{"taskType": "INPAINTING", "inPaintingParams": {"image": "aW1n", "text": "edit it", "maskImage": "bWFzaw=="}}"#,
        image: "b3V0",
    })
    .await;
    assert_eq!(inpainting.cost(), Some(0.25_f64));
    canvas_asynchronous().await;
    canvas_requires_an_image().await;
    failures(CANVAS_MODEL, &[(500, r#"{"message": "no"}"#, 500)]).await;
}

async fn canvas_variation() {
    let setup = EditSetup::new();
    let mut fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        vec!["aW1n".to_owned(), "dHdv".to_owned()]
            .into_iter()
            .map(EditImage::Text)
            .collect(),
        image_params(
            r#"{"aws_region_name": "us-east-1", "width": 512, "height": 512, "numberOfImages": 2}"#,
        ),
    );
    fixture.extra_headers = Some(vec![("x-betterlite-probe".to_owned(), "kept".to_owned())]);
    let run = setup.run(&fixture, answer(200, CANVAS_REPLY)).await;
    canvas_variation_request(&run);
    assert_eq!(run.edited().response.created, NOW_SECONDS);
    assert_eq!(
        run.edit_images(),
        vec!["b3V0".to_owned(), "b3V0Mg==".to_owned()]
    );
    assert_eq!(run.cost(), Some(0.5_f64));
}

fn canvas_variation_request(run: &Run) {
    assert_eq!(run.calls().len(), 1);
    assert_eq!(run.calls()[0], (ClientKind::Sync, None));
    assert_eq!(
        run.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.nova-canvas-v1:0/invoke"
    );
    // Only the first image reaches the transform, and the arm's extra headers reach the handler
    assert_eq!(
        run.edit_body(),
        image_params(
            r#"{"taskType": "IMAGE_VARIATION", "imageVariationParams": {"images": ["aW1n"], "text": "edit it"}, "imageGenerationConfig": {"width": 512, "height": 512, "numberOfImages": 2}}"#,
        )
    );
    assert_eq!(run.header("x-betterlite-probe"), Some("kept"));
}

async fn canvas_asynchronous() {
    let setup = EditSetup::new();
    let mut fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(REGION),
    );
    fixture.aimage_edit = true;
    fixture.timeout = Some(Duration::from_millis(2500));
    let run = setup
        .run(&fixture, answer(200, r#"{"images": ["b3V0"]}"#))
        .await;
    assert_eq!(run.calls().len(), 1);
    assert_eq!(
        run.calls()[0],
        (ClientKind::Async, Some(Duration::from_millis(2500)))
    );
    assert_eq!(
        run.edit_body(),
        image_params(
            r#"{"taskType": "IMAGE_VARIATION", "imageVariationParams": {"images": ["aW1n"], "text": "edit it"}}"#,
        )
    );
}

async fn canvas_requires_an_image() {
    let setup = EditSetup::new();
    let fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        Vec::new(),
        image_params(REGION),
    );
    let run = setup.run(&fixture, answer(200, CANVAS_REPLY)).await;
    assert!(matches!(run.error(), BedrockImageError::Edit(_)));
    assert_eq!(run.error().status(), None);
    assert!(run.requests().is_empty());
}

pub(super) async fn stability_edit() {
    stability_success().await;
    let prompt_only = edited_image(EditCase {
        model: STABILITY_MODEL,
        images: &[],
        optional_params: REGION,
        reply: r#"{"images": ["cXVpZXQ="]}"#,
        expected: r#"{"output_format": "png", "prompt": "edit it"}"#,
        image: "cXVpZXQ=",
    })
    .await;
    assert_eq!(prompt_only.cost(), Some(0.07_f64));
    failures(
        STABILITY_MODEL,
        &[
            (
                200_u16,
                r#"{"images": [], "finish_reasons": ["Filter reason"]}"#,
                400_u16,
            ),
            (500, r#"{"message": "no"}"#, 500),
        ],
    )
    .await;
}

async fn stability_success() {
    let setup = EditSetup::new();
    let fixture = EditFixture::new(
        Some(STABILITY_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(
            r#"{"aws_region_name": "us-east-1", "aspect_ratio": "16:9", "output_format": "jpeg", "seed": 7, "_response_format": "url"}"#,
        ),
    );
    let run = setup
        .run(&fixture, answer(200, r#"{"images": ["c3RhYg=="]}"#))
        .await;
    assert_eq!(
        run.edit_body(),
        image_params(
            r#"{"output_format": "jpeg", "prompt": "edit it", "image": "aW1n", "aspect_ratio": "16:9", "seed": 7}"#,
        )
    );
    assert_eq!(run.edited().response.created, NOW_SECONDS);
    assert_eq!(run.edit_images(), vec!["c3RhYg==".to_owned()]);
    assert_eq!(run.cost(), Some(0.07_f64));
}

pub(super) struct EditCase<'case> {
    pub(super) model: &'case str,
    pub(super) images: &'case [&'case str],
    pub(super) optional_params: &'case str,
    pub(super) reply: &'case str,
    pub(super) expected: &'case str,
    pub(super) image: &'case str,
}

// One edit whose request body and returned image the family decides; the caller reads its cost header
pub(super) async fn edited_image(case: EditCase<'_>) -> Run {
    let setup = EditSetup::new();
    let fixture = EditFixture::new(
        Some(case.model),
        Some("edit it"),
        case.images
            .iter()
            .map(|source| EditImage::Text((*source).to_owned()))
            .collect(),
        image_params(case.optional_params),
    );
    let run = setup.run(&fixture, answer(200, case.reply)).await;
    assert_eq!(run.edit_body(), image_params(case.expected));
    assert_eq!(run.edit_images(), vec![case.image.to_owned()]);
    run
}

// Each family's failing upstream answers: the status it answers with, its body, and the status it reports
async fn failures(model: &str, rows: &[(u16, &str, u16)]) {
    for &(upstream, body, reported) in rows {
        failed(model, upstream, body, reported).await;
    }
}

// An upstream answer the edit transform turns into an error with the reported status
pub(super) async fn failed(model: &str, upstream: u16, body: &str, reported: u16) {
    let setup = EditSetup::new();
    let fixture = EditFixture::new(
        Some(model),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(REGION),
    );
    let run = setup.run(&fixture, answer(upstream, body)).await;
    assert_eq!(run.error().status(), Some(reported));
    assert_eq!(run.requests().len(), 1);
}
