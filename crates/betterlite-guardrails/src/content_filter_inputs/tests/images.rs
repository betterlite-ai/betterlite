#![cfg(test)]

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Barrier;
use tokio::time::timeout;

use betterlite_config::sections::content_filter_records::ContentFilterAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::content_filter_inputs::error::ImageProcessingError;
use crate::content_filter_inputs::images::{
    IMAGE_DESCRIPTION_PROMPT, ImageDescriptionCompletion, ImageDescriptionReply,
    ImageDescriptionRequest, ImageDescriptionResponse, ImageScan, process_images,
};
use crate::guardrail_base::failure::GuardrailFailure;

use super::{Fixture, pattern_names, regex_pattern};

#[derive(Debug, thiserror::Error)]
#[error("the scripted completion failed")]
struct ScriptedError;

fn reply(content: Option<&str>) -> ImageDescriptionResponse {
    ImageDescriptionResponse {
        choices: vec![ImageDescriptionReply {
            content: content.map(str::to_owned),
        }],
    }
}

struct Scripted {
    content: Option<String>,
    requests: Mutex<Vec<ImageDescriptionRequest>>,
}

impl Scripted {
    fn record_request(&self, request: &ImageDescriptionRequest) {
        let mut requests = self.requests.lock().expect("the request list lock");
        requests.push(request.clone());
    }
}

#[async_trait]
impl ImageDescriptionCompletion for Scripted {
    type Error = ScriptedError;

    async fn describe(
        &self,
        request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        self.record_request(request);
        Ok(reply(self.content.as_deref()))
    }
}

struct Concurrent {
    barrier: Barrier,
    description: String,
    requests: Mutex<Vec<ImageDescriptionRequest>>,
}

#[async_trait]
impl ImageDescriptionCompletion for Concurrent {
    type Error = ScriptedError;

    async fn describe(
        &self,
        request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        self.requests
            .lock()
            .expect("the request list lock")
            .push(request.clone());
        self.barrier.wait().await;
        Ok(reply(Some(&self.description)))
    }
}

struct TwoChoices;

fn two_choices() -> ImageDescriptionResponse {
    ImageDescriptionResponse {
        choices: vec![
            ImageDescriptionReply {
                content: Some("first alpha".to_owned()),
            },
            ImageDescriptionReply {
                content: Some("second beta".to_owned()),
            },
        ],
    }
}

#[async_trait]
impl ImageDescriptionCompletion for TwoChoices {
    type Error = ScriptedError;

    async fn describe(
        &self,
        _request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        Ok(two_choices())
    }
}

struct NoChoices;

#[async_trait]
impl ImageDescriptionCompletion for NoChoices {
    type Error = ScriptedError;

    async fn describe(
        &self,
        _request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        Ok(ImageDescriptionResponse {
            choices: Vec::new(),
        })
    }
}

struct Failing;

#[async_trait]
impl ImageDescriptionCompletion for Failing {
    type Error = ScriptedError;

    async fn describe(
        &self,
        _request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        Err(ScriptedError)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct OrderedError(String);

struct FailingInOrder;

#[async_trait]
impl ImageDescriptionCompletion for FailingInOrder {
    type Error = OrderedError;

    async fn describe(
        &self,
        request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        let image_url = request.image_url.clone();
        Err(OrderedError(format!("failed for {image_url}")))
    }
}

#[tokio::test]
async fn image_description_fetching() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "alpha",
        "alpha",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let images = ["img-1".to_owned(), "img-2".to_owned()];
    let completion = Concurrent {
        barrier: Barrier::new(2),
        description: "an alpha scene".to_owned(),
        requests: Mutex::new(Vec::new()),
    };
    let scan = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    timeout(
        Duration::from_secs(10),
        process_images(&scan, Some(&completion), &mut detections),
    )
    .await
    .expect("both descriptions must be requested before either returns")
    .expect("the descriptions filter");
    let requests = completion.requests.lock().expect("the request list lock");
    assert_eq!(requests.len(), 2);
    let urls: Vec<&str> = requests
        .iter()
        .map(|request| request.image_url.as_str())
        .collect();
    assert_eq!(urls, ["img-1", "img-2"]);
    assert!(
        requests
            .iter()
            .all(|request| request.model == "vision-model"
                && request.system_prompt == IMAGE_DESCRIPTION_PROMPT)
    );
    assert_eq!(pattern_names(&detections), ["alpha", "alpha"]);
}

#[tokio::test]
async fn image_description_filtering_blocks_with_the_description() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "leak",
        "alpha",
        ContentFilterAction::Block,
    )]);
    let source = fixture.source();
    let images = ["img-1".to_owned()];
    let completion = Scripted {
        content: Some("an alpha scene".to_owned()),
        requests: Mutex::new(Vec::new()),
    };
    let scan = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    let error = process_images(&scan, Some(&completion), &mut detections)
        .await
        .unwrap_err();
    let GuardrailFailure::HttpObject { status, detail } = error.into_failure() else {
        panic!("a blocked description must raise the HTTP detail");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Content blocked: leak pattern detected (Image description): an alpha scene".to_owned()
        ))
    );
    assert_eq!(
        detail.get("pattern"),
        Some(&GuardrailValue::Text("leak".to_owned()))
    );
}

#[tokio::test]
async fn image_processing_skips_without_a_model_or_completion() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "alpha",
        "alpha",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let completion = Scripted {
        content: Some("an alpha scene".to_owned()),
        requests: Mutex::new(Vec::new()),
    };
    let images = ["img-1".to_owned()];
    let mut detections = Vec::new();
    let no_model = ImageScan {
        images: &images,
        image_model: None,
        source: &source,
    };
    process_images(&no_model, Some(&completion), &mut detections)
        .await
        .expect("a missing image model skips");
    let with_model = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    process_images(&with_model, None::<&Scripted>, &mut detections)
        .await
        .expect("a missing completion skips");
    assert!(
        completion
            .requests
            .lock()
            .expect("the request list lock")
            .is_empty()
    );
    assert!(detections.is_empty());
}

#[tokio::test]
async fn image_processing_skips_an_empty_image_list() {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let completion = Scripted {
        content: Some("an alpha scene".to_owned()),
        requests: Mutex::new(Vec::new()),
    };
    let no_images: [String; 0] = [];
    let empty = ImageScan {
        images: &no_images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    process_images(&empty, Some(&completion), &mut detections)
        .await
        .expect("an empty image list skips");
    assert!(
        completion
            .requests
            .lock()
            .expect("the request list lock")
            .is_empty()
    );
    assert!(detections.is_empty());
}

#[tokio::test]
async fn image_description_uses_the_first_choice_only() {
    let fixture = Fixture::with_patterns(vec![regex_pattern(
        "alpha",
        "alpha",
        ContentFilterAction::Mask,
    )]);
    let source = fixture.source();
    let images = ["img-1".to_owned()];
    let scan = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    process_images(&scan, Some(&TwoChoices), &mut detections)
        .await
        .expect("the first choice filters");
    assert_eq!(pattern_names(&detections), ["alpha"]);
}

#[tokio::test]
async fn image_processing_without_choices_fails() {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let images = ["img-1".to_owned()];
    let scan = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    let error = process_images(&scan, Some(&NoChoices), &mut detections)
        .await
        .unwrap_err();
    assert!(matches!(&error, ImageProcessingError::NoChoices));
    let GuardrailFailure::Other(message) = error.into_failure() else {
        panic!("a response without choices is not a block");
    };
    assert_eq!(message, "the image description response has no choices");
}

#[tokio::test]
async fn image_processing_completion_failure_propagates() {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let images = ["img-1".to_owned()];
    let scan = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    let error = process_images(&scan, Some(&Failing), &mut detections)
        .await
        .unwrap_err();
    assert!(matches!(&error, ImageProcessingError::Completion { .. }));
    let GuardrailFailure::Other(message) = error.into_failure() else {
        panic!("a failed completion is not a block");
    };
    assert_eq!(message, "the scripted completion failed");
}

#[tokio::test]
async fn image_processing_skips_empty_descriptions() {
    let fixture = Fixture::with_patterns(vec![
        regex_pattern("leak", "alpha", ContentFilterAction::Block),
        regex_pattern("blank", "^$", ContentFilterAction::Mask),
    ]);
    let source = fixture.source();
    let images = ["img-1".to_owned()];
    let scan = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    let missing = Scripted {
        content: None,
        requests: Mutex::new(Vec::new()),
    };
    process_images(&scan, Some(&missing), &mut detections)
        .await
        .expect("a missing description is skipped");
    let blank = Scripted {
        content: Some(String::new()),
        requests: Mutex::new(Vec::new()),
    };
    process_images(&scan, Some(&blank), &mut detections)
        .await
        .expect("an empty description is skipped");
    assert!(detections.is_empty());
}

#[tokio::test]
async fn image_processing_reports_the_first_failure_in_image_order() {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let images = ["img-1".to_owned(), "img-2".to_owned()];
    let scan = ImageScan {
        images: &images,
        image_model: Some("vision-model"),
        source: &source,
    };
    let mut detections = Vec::new();
    let error = process_images(&scan, Some(&FailingInOrder), &mut detections)
        .await
        .unwrap_err();
    let GuardrailFailure::Other(message) = error.into_failure() else {
        panic!("a failed completion is not a block");
    };
    assert_eq!(message, "failed for img-1");
}
