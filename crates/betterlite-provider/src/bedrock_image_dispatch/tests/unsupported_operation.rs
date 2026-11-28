use std::time::{Duration, SystemTime};

use betterlite_bedrock::image_edits::plan::EditImage;

use super::edit_fixture::{EditFixture, EditSetup};
use super::fixture::{answer, image_params, params, run, run_request};
use super::generate_fixture::GenerateFixture;
use crate::bedrock_image_dispatch::call::BedrockImageOperation;
use crate::bedrock_image_dispatch::error::BedrockImageError;
use crate::openai_media_dispatch::OpenAiMediaRequest;
use crate::operation::OperationRequest;

const STABILITY_MODEL: &str = "stability.stable-image-inpaint-v1:0";
const REGION: &str = r#"{"aws_region_name": "us-east-1"}"#;

// The Rust-only invariants: an arm this provider does not run, a request built for another provider, and an absent model
#[tokio::test]
async fn bedrock_image_dispatch_unsupported_operation() {
    unsupported_arm().await;
    mismatched_request().await;
    generation_without_a_model().await;
    edit_rejections().await;
}

async fn unsupported_arm() {
    let specified = run(
        BedrockImageOperation::Unsupported {
            operation: "image_edit",
        },
        answer(200, r#"{"images": []}"#),
    )
    .await;
    assert_eq!(
        specified.error().to_string(),
        "betterlite does not run the image_edit operation through the Bedrock image provider"
    );
    assert!(matches!(
        specified.error(),
        BedrockImageError::Unsupported {
            operation: "image_edit"
        }
    ));
    assert_eq!(specified.error().status(), None);
    assert!(specified.requests().is_empty());
}

async fn mismatched_request() {
    let mismatched = run_request(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        BedrockImageOperation::Unsupported {
            operation: "image_generation",
        },
        answer(200, r#"{"images": []}"#),
    )
    .await;
    assert_eq!(
        mismatched.error().to_string(),
        "the bedrock image composition runs only generation and edit operations"
    );
    assert!(matches!(
        mismatched.error(),
        BedrockImageError::UnsupportedOperation
    ));
    assert!(mismatched.requests().is_empty());
}

async fn generation_without_a_model() {
    let generation = GenerateFixture::new(None, "a cat", params(REGION));
    let generated = run(
        BedrockImageOperation::Generate(generation.operation()),
        answer(200, "{}"),
    )
    .await;
    assert_eq!(
        generated.error().to_string(),
        "Model needs to be set for bedrock"
    );
    assert!(matches!(
        generated.error(),
        BedrockImageError::ModelRequired
    ));
    assert!(generated.requests().is_empty());
}

// The edit arm refuses the absent model first and the clock before the epoch second
async fn edit_rejections() {
    let setup = EditSetup::new();
    let mut fixture = EditFixture::new(
        Some(STABILITY_MODEL),
        Some("edit it"),
        vec![EditImage::Text("aW1n".to_owned())],
        image_params(REGION),
    );
    fixture.model = None;
    let missing = setup.run(&fixture, answer(200, "{}")).await;
    assert_eq!(
        missing.error().to_string(),
        "Model needs to be set for bedrock"
    );
    assert!(matches!(missing.error(), BedrockImageError::ModelRequired));
    assert!(missing.requests().is_empty());

    fixture.model = Some(STABILITY_MODEL.to_owned());
    fixture.now = SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .expect("the Unix epoch has a second before it");
    let clock = setup.run(&fixture, answer(200, "{}")).await;
    assert_eq!(
        clock.error().to_string(),
        "the supplied clock is before the Unix epoch"
    );
    assert!(matches!(clock.error(), BedrockImageError::Clock));
    assert!(clock.requests().is_empty());
}
