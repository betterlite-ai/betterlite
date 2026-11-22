#![cfg(test)]

use std::collections::BTreeMap;
use std::str::from_utf8;
use std::time::Duration;

use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{
    ClientIdentity, ClientKind, LiteLlmParams, SessionKind,
};
use betterlite_azure::image_transport::ImageClock;
use betterlite_azure::images::generation::GeneratedImage;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::image_edits::file::ImageEditFile;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use super::call::{
    AzureMediaOperation, ImageEditOperation, ImageGenerationOperation,
    NativeTranscriptionOperation, TranscriptionOperation,
};
use super::error::AzureMediaError;
use super::parts::{form_parts, header_map};
use super::test_support::{Cache, Clock, Outcome, Reply, exchange, exchange_request};
use super::{AzureMediaResult, ImageEditOutcome};
use crate::openai_media_dispatch::OpenAiMediaRequest;
use crate::operation::{OperationRequest, OperationResult};

mod transcription;

const AZURE: &str = "https://example.openai.azure.com";
const API_KEY: &str = "sk-azure-fixture";
const DALL_E_BODY: &str =
    r#"{"created":1700000000,"data":[{"b64_json":"aW1hZ2U=","revised_prompt":"a cat"}]}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    ImageGeneration,
    ImageEdit,
    OpenAiSpeech,
    NativeSsml,
    Transcription,
}

#[rstest]
#[case::image_generation(Family::ImageGeneration)]
#[case::image_edit(Family::ImageEdit)]
#[case::openai_speech(Family::OpenAiSpeech)]
#[case::native_ssml(Family::NativeSsml)]
#[case::transcription(Family::Transcription)]
#[tokio::test]
async fn azure_media_dispatch_cases(#[case] family: Family) {
    match family {
        Family::ImageGeneration => image_generation().await,
        Family::ImageEdit => image_edit().await,
        Family::OpenAiSpeech => openai_speech().await,
        Family::NativeSsml => native_ssml().await,
        Family::Transcription => transcription::check().await,
    }
}

#[tokio::test]
async fn azure_media_dispatch_unsupported_operation() {
    let outcome = exchange_request(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        AzureMediaOperation::OpenAiSpeech,
        Vec::new(),
    )
    .await;
    assert!(matches!(
        outcome.result,
        Err(AzureMediaError::UnsupportedOperation)
    ));
    assert!(outcome.fixture.images().is_empty());
    assert!(outcome.fixture.outbound().is_empty());
}

fn key() -> Option<SecretString> {
    Some(SecretString::from(API_KEY))
}

fn generation(model: &str, version: &str, delegate_async: bool) -> ImageGenerationOperation {
    ImageGenerationOperation {
        model: Some(model.to_owned()),
        prompt: String::from("a cat"),
        timeout: None,
        optional_params: IndexMap::new(),
        headers: Vec::new(),
        api_key: key(),
        api_base: Some(AZURE.to_owned()),
        api_version: Some(version.to_owned()),
        azure_ad_token: None,
        provider: None,
        base_model: None,
        params: LiteLlmParams::default(),
        delegate_async,
        now_seconds: 1_700_000_000,
    }
}

fn image_result(outcome: &Outcome) -> &GeneratedImage {
    let Ok(OperationResult::AzureMedia(AzureMediaResult::ImageGeneration(image))) = &outcome.result
    else {
        panic!("the image generation arm returns its generated image");
    };
    image
}

fn edit_result(outcome: &Outcome) -> &ImageEditOutcome {
    let Ok(OperationResult::AzureMedia(AzureMediaResult::ImageEdit(edit))) = &outcome.result else {
        panic!("the image edit arm returns its outcome");
    };
    edit
}

fn error_of(outcome: &Outcome) -> &AzureMediaError {
    match &outcome.result {
        Err(error) => error,
        Ok(_) => panic!("the dispatch is expected to fail"),
    }
}

async fn image_generation() {
    sync_generation().await;
    polling_generation().await;
    async_generation().await;
    generation_failure().await;
}

// the sync leg sends the deployment-scoped body without the model and decodes the reply
async fn sync_generation() {
    let outcome = exchange(
        AzureMediaOperation::ImageGeneration(Box::new(generation(
            "dall-e-3",
            "2025-02-01-preview",
            false,
        ))),
        vec![Reply::json(200, DALL_E_BODY)],
    )
    .await;
    let image = image_result(&outcome);
    assert_eq!(image.reply.created, 1_700_000_000);
    assert_eq!(image.reply.data.len(), 1);
    assert_eq!(image.hidden_model, None);
    assert_eq!(outcome.logger.events, ["image_pre", "image_post"]);
    let sent = &outcome.fixture.images()[0];
    assert_eq!(sent.method, Method::POST);
    assert_eq!(
        sent.url,
        format!(
            "{AZURE}/openai/deployments/dall-e-3/images/generations?api-version=2025-02-01-preview"
        )
    );
    let body: Json = serde_json::from_slice(&sent.body).expect("the generation body is JSON");
    let Json::Object(fields) = body else {
        panic!("the generation body is an object");
    };
    assert!(fields.get("model").is_none());
    assert_eq!(
        fields.get("prompt"),
        Some(&Json::Text(String::from("a cat")))
    );
    assert_eq!(header(&sent.headers, "api-key"), Some(API_KEY));
}

// DALL-E-2's preview versions submit and then poll the operation-location
async fn polling_generation() {
    let terminal = r#"{"status":"succeeded","result":{"created":1700000000,"data":[{"b64_json":"aW1hZ2U="}]}}"#;
    let outcome = exchange(
        AzureMediaOperation::ImageGeneration(Box::new(generation(
            "dall-e-2",
            "2023-06-01-preview",
            false,
        ))),
        vec![
            Reply::json(200, "{}").with_header(
                "operation-location",
                "https://example.openai.azure.com/operations/7",
            ),
            Reply::json(200, terminal),
        ],
    )
    .await;
    let image = image_result(&outcome);
    assert_eq!(image.reply.data.len(), 1);
    let images = outcome.fixture.images();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].method, Method::POST);
    assert_eq!(
        images[0].url,
        format!("{AZURE}/openai/images/generations:submit?api-version=2023-06-01-preview")
    );
    assert_eq!(images[1].method, Method::GET);
    assert_eq!(
        images[1].url,
        "https://example.openai.azure.com/operations/7"
    );
}

// a delegated call takes the async leg and applies the family's reply defaults
async fn async_generation() {
    let outcome = exchange(
        AzureMediaOperation::ImageGeneration(Box::new(generation(
            "gpt-image-1",
            "2025-04-01-preview",
            true,
        ))),
        vec![Reply::json(200, DALL_E_BODY)],
    )
    .await;
    let image = image_result(&outcome);
    assert_eq!(image.reply.data.len(), 1);
    assert_eq!(image.reply.size.as_deref(), Some("1024x1024"));
    assert_eq!(outcome.logger.events, ["image_pre", "image_post"]);
}

async fn generation_failure() {
    let outcome = exchange(
        AzureMediaOperation::ImageGeneration(Box::new(generation(
            "dall-e-3",
            "2025-02-01-preview",
            false,
        ))),
        vec![Reply::json(500, "boom")],
    )
    .await;
    let error = error_of(&outcome);
    assert_eq!(error.status(), Some(500));
    assert!(matches!(error, AzureMediaError::Image(_)));
    assert!(error.to_string().contains("boom"));
}

fn edit_operation() -> ImageEditOperation {
    ImageEditOperation {
        model: String::from("gpt-image-1"),
        prompt: Some(String::from("make it blue")),
        image: Some(vec![Some(ImageEditFile {
            content: b"PNGDATA".to_vec(),
            name: Some(String::from("cat.png")),
        })]),
        mask: None,
        options: IndexMap::from([
            (String::from("size"), Json::Text(String::from("1024x1024"))),
            (String::from("n"), Json::Number(Number::from(2))),
        ]),
        headers: BTreeMap::new(),
        extra_headers: BTreeMap::new(),
        api_key: key(),
        api_base: Some(AZURE.to_owned()),
        api_version: Some(String::from("2025-04-01-preview")),
        provider: None,
        params: LiteLlmParams::default(),
        boundary: Uuid::from_u128(7),
        now_seconds: 1_700_000_000,
    }
}

async fn image_edit() {
    edit_success().await;
    edit_extra_headers().await;
    edit_failure().await;
    edit_bad_field().await;
    edit_boundary_collision().await;
    edit_bad_headers().await;
}

async fn edit_success() {
    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(edit_operation())),
        vec![Reply::json(200, DALL_E_BODY)],
    )
    .await;
    let edit = edit_result(&outcome);
    assert_eq!(edit.reply.data.len(), 1);
    assert_eq!(
        edit.url,
        format!(
            "{AZURE}/openai/deployments/gpt-image-1/images/edits?api-version=2025-04-01-preview"
        )
    );
    assert_eq!(edit.headers.get("api-key"), Some(&API_KEY.to_owned()));
    assert_eq!(
        edit.data.get("prompt"),
        Some(&Json::Text(String::from("make it blue")))
    );
    assert_eq!(edit.data.get("n"), Some(&Json::Number(Number::from(2))));
    let (field, file) = &edit.files[0];
    assert_eq!(field, "image[]");
    assert_eq!(file.filename.as_deref(), Some("cat.png"));
    let sent = &outcome.fixture.images()[0];
    let body = from_utf8(&sent.body).expect("the edit form body is UTF-8");
    assert!(body.contains("name=\"image[]\""));
    assert!(body.contains("filename=\"cat.png\""));
    assert!(!body.contains("name=\"model\""));
}

// the call's extra_headers update the validated headers after the auth header is set
async fn edit_extra_headers() {
    let operation = ImageEditOperation {
        extra_headers: BTreeMap::from([(String::from("api-key"), String::from("caller-key"))]),
        ..edit_operation()
    };
    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(operation)),
        vec![Reply::json(200, DALL_E_BODY)],
    )
    .await;
    let edit = edit_result(&outcome);
    assert_eq!(
        edit.headers.get("api-key").map(String::as_str),
        Some("caller-key")
    );
    assert_eq!(
        header(&outcome.fixture.images()[0].headers, "api-key"),
        Some("caller-key")
    );
}

// a non-2xx reply raises the upstream status with the reply text before any transform reads the body
async fn edit_failure() {
    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(edit_operation())),
        vec![Reply::json(400, "bad edit")],
    )
    .await;
    let error = error_of(&outcome);
    assert_eq!(error.status(), Some(400));
    assert!(matches!(error, AzureMediaError::EditStatus { .. }));
    assert_eq!(error.to_string(), "bad edit");

    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(edit_operation())),
        vec![Reply {
            status: 502,
            headers: HeaderMap::new(),
            body: vec![0xff],
        }],
    )
    .await;
    let error = error_of(&outcome);
    assert_eq!(error.status(), Some(502));
    assert_eq!(error.to_string(), "\u{fffd}");
}

async fn edit_bad_field() {
    let operation = ImageEditOperation {
        options: IndexMap::from([(
            String::from("size"),
            Json::List(vec![Json::Text(String::from("1024x1024"))]),
        )]),
        ..edit_operation()
    };
    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(operation)),
        Vec::new(),
    )
    .await;
    let error = error_of(&outcome);
    assert!(matches!(
        error,
        AzureMediaError::FormField { field } if field == "size"
    ));
    assert_eq!(error.status(), None);
    assert!(outcome.fixture.images().is_empty());
}

// a form value that carries the boundary cannot be encoded, and the send never happens
async fn edit_boundary_collision() {
    let boundary = Uuid::from_u128(7);
    let operation = ImageEditOperation {
        prompt: Some(format!("--{boundary}")),
        boundary,
        ..edit_operation()
    };
    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(operation)),
        Vec::new(),
    )
    .await;
    let error = error_of(&outcome);
    assert!(matches!(error, AzureMediaError::Transport(_)));
    assert_eq!(error.status(), None);
    assert!(outcome.fixture.images().is_empty());
}

// a supplied header that cannot reach httpx fails before the send
async fn edit_bad_headers() {
    for extra in [
        BTreeMap::from([(String::from("bad header"), String::from("value"))]),
        BTreeMap::from([(String::from("x-bad"), String::from("a\nb"))]),
    ] {
        let operation = ImageEditOperation {
            extra_headers: extra,
            ..edit_operation()
        };
        let outcome = exchange(
            AzureMediaOperation::ImageEdit(Box::new(operation)),
            Vec::new(),
        )
        .await;
        let error = error_of(&outcome);
        assert!(error.status().is_none());
        assert!(matches!(
            error,
            AzureMediaError::HeaderName { .. } | AzureMediaError::HeaderValue { .. }
        ));
        assert!(outcome.fixture.images().is_empty());
    }
}

async fn openai_speech() {
    let outcome = exchange(AzureMediaOperation::OpenAiSpeech, Vec::new()).await;
    let error = error_of(&outcome);
    assert!(matches!(
        error,
        AzureMediaError::Unavailable {
            operation: "openai_speech"
        }
    ));
    assert!(error.to_string().contains("openai_speech"));
    assert_eq!(error.status(), None);
    assert!(outcome.fixture.images().is_empty());
    assert!(outcome.fixture.outbound().is_empty());
}

async fn native_ssml() {
    let outcome = exchange(AzureMediaOperation::NativeSsml, Vec::new()).await;
    let error = error_of(&outcome);
    assert!(matches!(
        error,
        AzureMediaError::Unavailable {
            operation: "native_ssml"
        }
    ));
    assert!(outcome.fixture.images().is_empty());
    assert!(outcome.fixture.outbound().is_empty());
}

#[test]
fn form_parts_render_the_httpx_field_text() {
    let data = IndexMap::from([
        (String::from("null"), Json::Null),
        (String::from("true"), Json::Bool(true)),
        (String::from("false"), Json::Bool(false)),
        (String::from("number"), Json::Number(Number::from(2))),
        (String::from("text"), Json::Text(String::from("hello"))),
    ]);
    let parts = form_parts(&data).expect("the scalar fields encode");
    let field = |name: &str| {
        parts
            .iter()
            .find(|part| part.name == name)
            .expect("the form field is present")
            .data
            .clone()
    };
    assert_eq!(field("null"), b"");
    assert_eq!(field("true"), b"true");
    assert_eq!(field("false"), b"false");
    assert_eq!(field("number"), b"2");
    assert_eq!(field("text"), b"hello");
}

#[rstest]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn form_parts_reject_non_scalar_fields(#[case] value: Json) {
    let fields = IndexMap::from([(String::from("field"), value)]);
    let error = form_parts(&fields).expect_err("the field is not text");
    assert!(matches!(error, AzureMediaError::FormField { .. }));
}

#[test]
fn header_map_rejects_unusable_names_and_values() {
    let name = header_map(&BTreeMap::from([(
        String::from("bad header"),
        String::from("value"),
    )]));
    assert!(matches!(name, Err(AzureMediaError::HeaderName { .. })));
    let value = header_map(&BTreeMap::from([(
        String::from("x-bad"),
        String::from("a\nb"),
    )]));
    assert!(matches!(value, Err(AzureMediaError::HeaderValue { .. })));
    let good = header_map(&BTreeMap::from([(
        String::from("api-key"),
        String::from("secret"),
    )]));
    good.expect("a valid header name and value are accepted");
}

fn header<'value>(headers: &'value HeaderMap, name: &str) -> Option<&'value str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn audio_file() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"abc".to_vec(),
        filename: String::from("a.wav"),
        content_type: String::from("audio/wav"),
    }
}

fn transcription_operation(atranscription: bool) -> TranscriptionOperation {
    TranscriptionOperation {
        input: String::from("a.wav"),
        file: audio_file(),
        boundary: Uuid::from_u128(11),
        model: String::from("whisper"),
        optional_params: IndexMap::new(),
        atranscription,
        api_version: Some(String::from("2024-06-01")),
        api_key: key(),
        api_base: Some(AZURE.to_owned()),
        azure_ad_token: None,
        max_retries: None,
        timeout: Duration::from_secs(30),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: None,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    }
}

fn native_operation() -> NativeTranscriptionOperation {
    let language = ("language".to_owned(), Json::Text("en-US".to_owned()));
    NativeTranscriptionOperation {
        filename: "a.wav".to_owned(),
        content: b"abc".to_vec(),
        optional_params: IndexMap::from([language]),
        headers: BTreeMap::new(),
        api_key: None,
        environment_key: Some(SecretString::from("speech-key")),
        api_base: Some("https://westus.api.cognitive.microsoft.com".to_owned()),
        environment_base: None,
        timeout: Duration::from_secs(30),
    }
}

// the fixture clock stays at one fixed instant, so polling sees no elapsed time
#[test]
fn fixture_clock_is_pinned() {
    assert_eq!(Clock.now(), Duration::from_secs(1));
}

#[test]
fn fixture_cache_round_trips() {
    let mut cache = Cache::default();
    let identity = ClientIdentity {
        is_async: true,
        hashed_api_key: Some(String::from("key")),
        hashed_ad_token: None,
        provider_identity: String::from("provider"),
    };
    assert_eq!(cache.get(&identity), None);
    cache.set(identity.clone(), ClientKind::AsyncAzureOpenAi, false);
    assert_eq!(cache.get(&identity), Some(ClientKind::AsyncAzureOpenAi));
}
