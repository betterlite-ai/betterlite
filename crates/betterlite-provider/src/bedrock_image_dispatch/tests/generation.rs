use std::time::Duration;

use betterlite_bedrock::image_call::transport::ClientKind;
use betterlite_models::decisions::Json;

use super::fixture::{GenerateFixture, NOW_SECONDS, Run, answer, params, run};
use crate::bedrock_image_dispatch::call::BedrockImageOperation;
use crate::bedrock_image_dispatch::error::BedrockImageError;

const TITAN_MODEL: &str = "amazon.titan-image-generator-v2:0";
const TITAN_BODY: &str = r#"{"images": ["dGl0YW4=", "dGl0YW4y"]}"#;
const STABILITY1_MODEL: &str = "stability.stable-diffusion-xl-v1";
const STABILITY3_MODEL: &str = "stability.sd3-large-v1:0";
const CANVAS_MODEL: &str = "amazon.nova-canvas-v1:0";
const REGION: &str = r#"{"aws_region_name": "us-east-1"}"#;

pub(super) async fn titan_image() {
    titan_success().await;
    titan_asynchronous().await;
    titan_provider_error().await;
}

async fn titan_success() {
    let mut fixture = GenerateFixture::new(
        Some(TITAN_MODEL),
        "a cat",
        params(
            r#"{"aws_region_name": "us-east-1", "imageGenerationConfig": {"width": 1024, "height": 512, "numberOfImages": 2}}"#,
        ),
    );
    fixture.extra_headers = Some(vec![(
        "x-betterlite-probe".to_owned(),
        "dropped".to_owned(),
    )]);
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(200, TITAN_BODY),
    )
    .await;
    titan_request(&run);
    assert_eq!(
        run.images(),
        vec!["dGl0YW4".to_owned(), "dGl0YW4y".to_owned()]
    );
    assert_eq!(run.post_call(), Some(&Json::Text(TITAN_BODY.to_owned())));
    assert_eq!(run.input(), Some(&Json::Text("a cat".to_owned())));
}

fn titan_request(run: &Run) {
    assert_eq!(run.calls().len(), 1);
    assert_eq!(run.calls()[0], (ClientKind::Sync, None));
    assert_eq!(
        run.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.titan-image-generator-v2:0/invoke"
    );
    assert_eq!(
        run.generation_body(),
        params(
            r#"{"taskType": "TEXT_IMAGE", "textToImageParams": {"text": "a cat"}, "imageGenerationConfig": {"width": 1024, "height": 512, "numberOfImages": 2}}"#,
        )
    );
    assert_eq!(run.created(), Some(NOW_SECONDS));
    // LiteLLM's generation arm reads extra_headers and forwards neither it nor headers to the handler
    assert_eq!(run.header("x-betterlite-probe"), None);
    assert!(
        run.authorization()
            .is_some_and(|value| { value.starts_with("AWS4-HMAC-SHA256") })
    );
}

async fn titan_asynchronous() {
    let mut fixture = GenerateFixture::new(Some(TITAN_MODEL), "a cat", params(REGION));
    fixture.aimg_generation = true;
    fixture.timeout = Some(Duration::from_millis(2500));
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(200, r#"{"images": ["b25l"]}"#),
    )
    .await;
    assert_eq!(run.calls().len(), 1);
    assert_eq!(
        run.calls()[0],
        (ClientKind::Async, Some(Duration::from_millis(2500)))
    );
    assert_eq!(
        run.generation_body(),
        params(
            r#"{"taskType": "TEXT_IMAGE", "textToImageParams": {"text": "a cat"}, "imageGenerationConfig": {}}"#,
        )
    );
    assert_eq!(run.images(), vec!["b25l".to_owned()]);
}

async fn titan_provider_error() {
    let fixture = GenerateFixture::new(Some(TITAN_MODEL), "a cat", params(REGION));
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(429, r#"{"message": "slow down"}"#),
    )
    .await;
    let error = run.error();
    assert_eq!(error.status(), Some(429));
    assert!(matches!(error, BedrockImageError::Generate(_)));
    assert_eq!(run.requests().len(), 1);
    assert_eq!(run.post_call(), None);
    assert_eq!(run.input(), Some(&Json::Text("a cat".to_owned())));
}

pub(super) async fn stability_image() {
    stability1_success().await;
    stability3_success().await;
    stability3_finish_reason().await;
    stability1_provider_error().await;
}

async fn stability1_success() {
    let fixture = GenerateFixture::new(
        Some(STABILITY1_MODEL),
        "a cat",
        params(r#"{"aws_region_name": "us-east-1", "width": 1024, "height": 1024}"#),
    );
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(200, r#"{"artifacts": [{"base64": "czE="}]}"#),
    )
    .await;
    let body = run.generation_body();
    assert_eq!(
        body,
        params(
            r#"{"text_prompts": [{"text": "a cat", "weight": 1}], "width": 1024, "height": 1024}"#,
        )
    );
    assert!(!body.contains_key("prompt"));
    assert_eq!(run.images(), vec!["czE=".to_owned()]);
    assert_eq!(run.created(), Some(NOW_SECONDS));
}

async fn stability3_success() {
    let fixture = GenerateFixture::new(
        Some(STABILITY3_MODEL),
        "a cat",
        params(r#"{"aws_region_name": "us-east-1", "aspect_ratio": "16:9", "seed": 7}"#),
    );
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(200, r#"{"images": ["czM="]}"#),
    )
    .await;
    let body = run.generation_body();
    assert_eq!(
        body,
        params(r#"{"prompt": "a cat", "aspect_ratio": "16:9", "seed": 7}"#)
    );
    assert!(!body.contains_key("text_prompts"));
    assert_eq!(run.images(), vec!["czM=".to_owned()]);
}

async fn stability3_finish_reason() {
    let fixture = GenerateFixture::new(Some(STABILITY3_MODEL), "a cat", params(REGION));
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(
            200,
            r#"{"images": [], "finish_reasons": ["Content filtered", ""]}"#,
        ),
    )
    .await;
    assert_eq!(run.error().status(), Some(400));
    // The leaf logs the reply before it judges the finish reasons
    assert_eq!(
        run.post_call(),
        Some(&Json::Text(
            r#"{"images": [], "finish_reasons": ["Content filtered", ""]}"#.to_owned()
        ))
    );
}

async fn stability1_provider_error() {
    let fixture = GenerateFixture::new(Some(STABILITY1_MODEL), "a cat", params(REGION));
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(400, r#"{"message": "bad request"}"#),
    )
    .await;
    assert_eq!(run.error().status(), Some(400));
    assert_eq!(run.requests().len(), 1);
}

pub(super) async fn canvas_image() {
    canvas_success().await;
    canvas_provider_error().await;
}

async fn canvas_success() {
    let fixture = GenerateFixture::new(
        Some(CANVAS_MODEL),
        "a cat",
        params(
            r#"{"aws_region_name": "us-east-1", "width": 512, "height": 512, "numberOfImages": 2, "quality": "premium"}"#,
        ),
    );
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(200, r#"{"images": ["bm92YQ=="]}"#),
    )
    .await;
    assert_eq!(
        run.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.nova-canvas-v1:0/invoke"
    );
    assert_eq!(
        run.generation_body(),
        params(
            r#"{"textToImageParams": {"text": "a cat"}, "taskType": "TEXT_IMAGE", "imageGenerationConfig": {"width": 512, "height": 512, "numberOfImages": 2, "quality": "premium"}}"#,
        )
    );
    assert_eq!(run.images(), vec!["bm92YQ==".to_owned()]);
    assert_eq!(run.created(), Some(NOW_SECONDS));
}

async fn canvas_provider_error() {
    let fixture = GenerateFixture::new(Some(CANVAS_MODEL), "a cat", params(REGION));
    let run = run(
        BedrockImageOperation::Generate(fixture.operation()),
        answer(500, r#"{"message": "the upstream failed"}"#),
    )
    .await;
    assert_eq!(run.error().status(), Some(500));
    assert_eq!(run.requests().len(), 1);
}
