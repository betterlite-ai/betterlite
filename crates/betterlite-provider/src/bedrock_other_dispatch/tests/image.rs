use betterlite_bedrock::canvas_results::COST_HEADER;
use betterlite_bedrock::image_call::plan::ImageEnvironment;
use betterlite_bedrock::image_edits::plan::{EditEnvironment, EditImage, EditPlan};
use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::transcription::error::CallError;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::ImageParams;

use super::fixture::{Fixture, Outcome, Unavailable, image_params, run, value};
use crate::bedrock_image_dispatch::BedrockImageResult;
use crate::bedrock_image_dispatch::call::{
    BedrockImageOperation, EditOperation, GenerateOperation,
};
use crate::bedrock_image_dispatch::test_support::Catalog;
use crate::bedrock_other_dispatch::call::BedrockOtherOperation;
use crate::bedrock_other_dispatch::error::BedrockOtherError;
use crate::bedrock_other_dispatch::test_support::client::Reply;
use crate::bedrock_other_dispatch::test_support::{NOW_SECONDS, credentials, instant};

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const CANVAS: &str = "amazon.nova-canvas-v1:0";
const GENERATION: &str = r#"{"aws_region_name": "us-east-1", "width": 512, "height": 512, "numberOfImages": 2, "quality": "premium"}"#;
const EDITING: &str =
    r#"{"aws_region_name": "us-east-1", "width": 512, "height": 512, "numberOfImages": 2}"#;
const MODEL_COST: &str = r#"{"amazon.nova-canvas-v1:0": {"litellm_provider": "bedrock", "supports_nova_canvas_image_edit": true, "output_cost_per_image": 0.25}}"#;

fn scripted(replies: &[Reply]) -> Fixture {
    Fixture::new(replies, credentials(ACCESS_KEY, SECRET_KEY))
}

fn model_params(source: &str) -> Params {
    serde_json::from_str(source).expect("the generation params are a JSON object")
}

fn generation<'call>(
    prompt: &'call str,
    optional: &'call str,
) -> BedrockOtherOperation<'call, 'call, 'call, Unavailable, super::fixture::Binding> {
    BedrockOtherOperation::Image(BedrockImageOperation::Generate(GenerateOperation {
        model: Some(CANVAS),
        prompt,
        optional_params: model_params(optional),
        extra_headers: None,
        api_base: None,
        api_key: None,
        timeout: None,
        aimg_generation: false,
        environment: ImageEnvironment::default(),
        now: instant(NOW_SECONDS).expect("the fixture instant"),
        created: NOW_SECONDS,
    }))
}

fn editing<'call, 'cost, 'context>(
    prompt: &'call str,
    images: &'call [EditImage],
    optional: ImageParams,
    model_cost: &'cost ParamMap,
    context: &'context ResolutionContext<'context>,
) -> BedrockOtherOperation<'call, 'cost, 'context, Unavailable, super::fixture::Binding> {
    BedrockOtherOperation::Image(BedrockImageOperation::Edit(EditOperation {
        model: Some(CANVAS),
        prompt: Some(prompt),
        images,
        optional_params: optional,
        extra_headers: None,
        api_base: None,
        api_key: None,
        timeout: None,
        aimage_edit: false,
        environment: EditEnvironment::default(),
        now: instant(NOW_SECONDS).expect("the fixture instant"),
        plan: EditPlan {
            model_cost,
            resolution: context,
        },
    }))
}

// One Canvas generation round trip: the leaf's body, endpoint, base64 images and creation time
async fn canvas_generation() {
    let fixture = scripted(&[Reply::status(200, r#"{"images": ["bm92YQ=="]}"#)]);
    let outcome = run(&fixture, generation("a cat", GENERATION)).await;
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.nova-canvas-v1:0/invoke"
    );
    assert_eq!(
        outcome.body(0),
        value(
            r#"{"textToImageParams": {"text": "a cat"}, "taskType": "TEXT_IMAGE", "imageGenerationConfig": {"width": 512, "height": 512, "numberOfImages": 2, "quality": "premium"}}"#
        )
    );
    let response = match outcome.image() {
        BedrockImageResult::Generate(response) => response.as_ref(),
        BedrockImageResult::Edit(_other) => panic!("the generation arm returns no edit reply"),
    };
    let images: Vec<&str> = response
        .data
        .iter()
        .map(|image| image.b64_json.as_str())
        .collect();
    assert_eq!(images, ["bm92YQ=="]);
    assert_eq!(response.created, Some(NOW_SECONDS));
}

// One Canvas edit round trip: only the first image reaches the variation body, and the reply keeps its cost
async fn canvas_edit() {
    let fixture = scripted(&[Reply::status(200, r#"{"images": ["b3V0"]}"#)]);
    let model_cost: ParamMap = serde_json::from_str(MODEL_COST).expect("the model cost map");
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let images = vec![
        EditImage::Text(String::from("aW1n")),
        EditImage::Text(String::from("dHdv")),
    ];
    let outcome = run(
        &fixture,
        editing(
            "edit it",
            &images,
            image_params(EDITING),
            &model_cost,
            &context,
        ),
    )
    .await;
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(
        outcome.body(0),
        value(
            r#"{"taskType": "IMAGE_VARIATION", "imageVariationParams": {"images": ["aW1n"], "text": "edit it"}, "imageGenerationConfig": {"width": 512, "height": 512, "numberOfImages": 2}}"#
        )
    );
    edit_reply(&outcome);
}

fn edit_reply(outcome: &Outcome<CallError>) {
    let reply = match outcome.image() {
        BedrockImageResult::Edit(reply) => reply,
        BedrockImageResult::Generate(_other) => panic!("the edit arm returns no generated reply"),
    };
    let returned: Vec<&str> = reply
        .response
        .data
        .iter()
        .flatten()
        .filter_map(|image| image.b64_json.as_deref())
        .collect();
    assert_eq!(returned, ["b3V0"]);
    assert_eq!(reply.response.created, NOW_SECONDS);
    assert_eq!(
        reply.additional_headers.get(COST_HEADER),
        Some(&value("0.25"))
    );
}

// The leaf's provider error reaches the caller as the image arm's failure, for generation and for edits
async fn leaf_provider_errors() {
    let failed = scripted(&[Reply::status(500, r#"{"message": "no"}"#)]);
    let outcome = run(&failed, generation("a cat", GENERATION)).await;
    assert!(matches!(outcome.error(), BedrockOtherError::Image(_)));
    assert_eq!(outcome.error().status(), Some(500));
    assert_eq!(outcome.requests.len(), 1);

    let refused = scripted(&[Reply::status(400, r#"{"message": "the edit failed"}"#)]);
    let model_cost: ParamMap = serde_json::from_str(MODEL_COST).expect("the model cost map");
    let catalog = Catalog::empty().expect("the empty catalog parses");
    let context = catalog.context();
    let images = vec![EditImage::Text(String::from("aW1n"))];
    let edit_outcome = run(
        &refused,
        editing(
            "edit it",
            &images,
            image_params(EDITING),
            &model_cost,
            &context,
        ),
    )
    .await;
    assert!(matches!(edit_outcome.error(), BedrockOtherError::Image(_)));
    assert_eq!(edit_outcome.error().status(), Some(400));
    assert_eq!(edit_outcome.requests.len(), 1);
}

pub(super) async fn case() {
    canvas_generation().await;
    canvas_edit().await;
    leaf_provider_errors().await;
}
