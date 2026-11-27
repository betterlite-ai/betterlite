use std::time::Duration;

use betterlite_bedrock::image_call::transport::ClientKind;
use betterlite_bedrock::image_edits::plan::EditImage;

use super::fixture::{
    Catalog, EditFixture, EditPlan, NOW_SECONDS, Run, answer, image_params, model_cost, run,
};
use crate::bedrock_image_dispatch::call::BedrockImageOperation;
use crate::bedrock_image_dispatch::error::BedrockImageError;

const CANVAS_MODEL: &str = "amazon.nova-canvas-v1:0";
const STABILITY_MODEL: &str = "stability.stable-image-inpaint-v1:0";
const CANVAS_REPLY: &str = r#"{"images": ["b3V0", "b3V0Mg=="]}"#;
const REGION: &str = r#"{"aws_region_name": "us-east-1"}"#;

pub(super) async fn canvas_edit() {
    canvas_variation().await;
    canvas_inpainting().await;
    canvas_asynchronous().await;
    canvas_requires_an_image().await;
    canvas_provider_error().await;
}

async fn canvas_variation() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let mut fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        vec![
            EditImage::Text("aW1n".to_owned()),
            EditImage::Text("dHdv".to_owned()),
        ],
        image_params(
            r#"{"aws_region_name": "us-east-1", "width": 512, "height": 512, "numberOfImages": 2}"#,
        ),
    );
    fixture.extra_headers = Some(vec![("x-betterlite-probe".to_owned(), "kept".to_owned())]);
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(200, CANVAS_REPLY),
    )
    .await;
    canvas_variation_request(&run);
    assert_eq!(run.edited().response.created, NOW_SECONDS);
    assert_eq!(
        run.edit_images(),
        vec!["b3V0".to_owned(), "b3V0Mg==".to_owned()]
    );
    assert_eq!(run.cost(), Some(0.5));
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

async fn canvas_inpainting() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(r#"{"aws_region_name": "us-east-1", "mask": "bWFzaw=="}"#),
    );
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(200, r#"{"images": ["b3V0"]}"#),
    )
    .await;
    assert_eq!(
        run.edit_body(),
        image_params(
            r#"{"taskType": "INPAINTING", "inPaintingParams": {"image": "aW1n", "text": "edit it", "maskImage": "bWFzaw=="}}"#,
        )
    );
    assert_eq!(run.edit_images(), vec!["b3V0".to_owned()]);
    assert_eq!(run.cost(), Some(0.25));
}

async fn canvas_asynchronous() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let mut fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(REGION),
    );
    fixture.aimage_edit = true;
    fixture.timeout = Some(Duration::from_millis(2500));
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(200, r#"{"images": ["b3V0"]}"#),
    )
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
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        Vec::new(),
        image_params(REGION),
    );
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(200, CANVAS_REPLY),
    )
    .await;
    assert!(matches!(run.error(), BedrockImageError::Edit(_)));
    assert_eq!(run.error().status(), None);
    assert!(run.requests().is_empty());
}

async fn canvas_provider_error() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let fixture = EditFixture::new(
        Some(CANVAS_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(REGION),
    );
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(500, r#"{"message": "no"}"#),
    )
    .await;
    assert_eq!(run.error().status(), Some(500));
    assert_eq!(run.requests().len(), 1);
}

pub(super) async fn stability_edit() {
    stability_success().await;
    stability_without_an_image().await;
    stability_finish_reason().await;
    stability_provider_error().await;
}

async fn stability_success() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let fixture = EditFixture::new(
        Some(STABILITY_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(
            r#"{"aws_region_name": "us-east-1", "aspect_ratio": "16:9", "output_format": "jpeg", "seed": 7, "_response_format": "url"}"#,
        ),
    );
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(200, r#"{"images": ["c3RhYg=="]}"#),
    )
    .await;
    assert_eq!(
        run.edit_body(),
        image_params(
            r#"{"output_format": "jpeg", "prompt": "edit it", "image": "aW1n", "aspect_ratio": "16:9", "seed": 7}"#,
        )
    );
    assert_eq!(run.edited().response.created, NOW_SECONDS);
    assert_eq!(run.edit_images(), vec!["c3RhYg==".to_owned()]);
    assert_eq!(run.cost(), Some(0.07));
}

async fn stability_without_an_image() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let fixture = EditFixture::new(
        Some(STABILITY_MODEL),
        Some("edit it"),
        Vec::new(),
        image_params(REGION),
    );
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(200, r#"{"images": ["cXVpZXQ="]}"#),
    )
    .await;
    assert_eq!(
        run.edit_body(),
        image_params(r#"{"output_format": "png", "prompt": "edit it"}"#)
    );
    assert_eq!(run.edit_images(), vec!["cXVpZXQ=".to_owned()]);
}

async fn stability_finish_reason() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let fixture = EditFixture::new(
        Some(STABILITY_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(REGION),
    );
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(
            200,
            r#"{"images": [], "finish_reasons": ["Filter reason"]}"#,
        ),
    )
    .await;
    assert_eq!(run.error().status(), Some(400));
    assert_eq!(run.requests().len(), 1);
}

async fn stability_provider_error() {
    let model_cost = model_cost();
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let fixture = EditFixture::new(
        Some(STABILITY_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(REGION),
    );
    let plan = EditPlan {
        model_cost: &model_cost,
        resolution: &context,
    };
    let run = run(
        BedrockImageOperation::Edit(fixture.operation(plan)),
        answer(500, r#"{"message": "no"}"#),
    )
    .await;
    assert_eq!(run.error().status(), Some(500));
}
