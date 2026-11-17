use std::time::Duration;

use betterlite_openai::image_generation::call::ImageGenerationCall;
use http::Method;
use secrecy::SecretString;

use super::fixture::{Fixture, Recorder, json_body, media_error, media_result, options};
use crate::openai_media_dispatch::call::OpenAiMediaOperation;
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{OpenAiMediaRequest, OpenAiMediaResult};
use crate::operation::{OperationRequest, OperationResult};

const API_BASE: &str = "https://images.test/v1";
const NOW: i64 = 1_700_000_000;
const DALL_E_REPLY: &[u8] =
    br#"{"created":1700000001,"data":[{"b64_json":"ZGFsbC1l"}],"quality":"hd","size":"1024x1024"}"#;
const GPT_REPLY: &[u8] = br#"{"created":1700000002,"data":[{"b64_json":"Z3B0"}],"background":"transparent","output_format":"webp","size":"1024x1024"}"#;

fn call(model: &str, optional: &str) -> ImageGenerationCall {
    ImageGenerationCall {
        model: String::from(model),
        prompt: String::from("a red bicycle"),
        optional_params: options(optional),
        api_key: Some(SecretString::from("sk-image")),
        api_base: Some(String::from(API_BASE)),
        organization: None,
        headers: None,
        timeout: Duration::from_secs(60),
        delegate_async: false,
        now_seconds: NOW,
    }
}

async fn run(
    call: ImageGenerationCall,
    fixture: &Fixture,
) -> Result<OperationResult, OpenAiMediaError> {
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::ImageGeneration(Box::new(call)),
        fixture,
        &mut logger,
    )
    .await
}

pub(super) async fn check() {
    dall_e().await;
    gpt_image().await;
    failure().await;
    two_dispatches_stay_apart().await;
}

async fn dall_e() {
    let fixture = Fixture::new(200, DALL_E_REPLY);
    let result = media_result(
        run(
            call("dall-e-3", r#"{"size":"1024x1024","quality":"hd"}"#),
            &fixture,
        )
        .await,
    );
    let OpenAiMediaResult::ImageGeneration(outcome) = result else {
        panic!("a generation reply is never read as an edit");
    };
    assert_eq!(fixture.request_count(), 1);
    let request = fixture
        .take_request()
        .expect("one dispatch sends one request");
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "https://images.test/v1/images/generations"
    );
    assert_eq!(
        json_body(&request),
        options(
            r#"{"model":"dall-e-3","prompt":"a red bicycle","size":"1024x1024","quality":"hd"}"#
        )
    );
    assert_eq!(outcome.reply.created, 1_700_000_001);
    assert_eq!(outcome.reply.data.len(), 1);
    assert_eq!(outcome.reply.data[0].b64_json.as_deref(), Some("ZGFsbC1l"));
    assert_eq!(outcome.reply.quality.as_deref(), Some("hd"));
}

async fn gpt_image() {
    let fixture = Fixture::new(200, GPT_REPLY);
    let result = media_result(
        run(
            call(
                "gpt-image-1",
                r#"{"background":"transparent","output_format":"webp","size":"1024x1024"}"#,
            ),
            &fixture,
        )
        .await,
    );
    let OpenAiMediaResult::ImageGeneration(outcome) = result else {
        panic!("a generation reply is never read as an edit");
    };
    let request = fixture
        .take_request()
        .expect("one dispatch sends one request");
    assert_eq!(
        json_body(&request),
        options(
            r#"{"model":"gpt-image-1","prompt":"a red bicycle","background":"transparent","output_format":"webp","size":"1024x1024"}"#
        )
    );
    assert_eq!(outcome.reply.output_format.as_deref(), Some("webp"));
    assert_eq!(outcome.reply.background.as_deref(), Some("transparent"));
    assert_eq!(outcome.reply.size.as_deref(), Some("1024x1024"));
}

async fn two_dispatches_stay_apart() {
    let fixture = Fixture::new(200, DALL_E_REPLY);
    let first = media_result(run(call("dall-e-3", "{}"), &fixture).await);
    let second = media_result(run(call("dall-e-3", "{}"), &fixture).await);
    assert!(matches!(first, OpenAiMediaResult::ImageGeneration(_)));
    assert!(matches!(second, OpenAiMediaResult::ImageGeneration(_)));
    assert_eq!(fixture.request_count(), 2);
    assert!(fixture.take_request().is_none());
}

async fn failure() {
    let fixture = Fixture::new(500, br#"{"error":{"message":"upstream exploded"}}"#);
    let error = media_error(run(call("dall-e-3", "{}"), &fixture).await);
    let OpenAiMediaError::ImageGeneration(failure) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    assert_eq!(failure.status(), Some(500));
    assert!(failure.mapped());
    assert_eq!(failure.to_string(), "upstream exploded");
}
