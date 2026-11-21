#![cfg(test)]
#![expect(
    clippy::too_many_lines,
    reason = "one test per documented behavior, each asserting the whole recorded call"
)]

use std::sync::Arc;

use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use self::support::{
    CustomHandler, CustomSource, FailingLogger, NoCustom, Provider, RecordingLogger, Source, World,
    custom_call, gpt_image_tag, image, redactor, response, settings, upload,
};
use super::call::{DEFAULT_MODEL, DEFAULT_REQUEST_TIMEOUT, ImageEditClientKind, ImageInput};
use super::entry::ImageEditEntry;
use super::error::ImageEditError;
use super::service::{ImageEdits, branch_timeout};
use crate::api::image_input_options::selection::OptionalParamsPolicy;

mod support;

#[tokio::test]
async fn edit_openai_upload() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(
        &[
            "image",
            "prompt",
            "background",
            "input_fidelity",
            "mask",
            "model",
            "n",
            "quality",
            "response_format",
            "size",
            "user",
        ],
        true,
    ));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut logger = RecordingLogger::new();
    let answer = ImageEditEntry {
        operation: &service,
        logger: &mut logger,
        redactor: &redactor(),
    }
    .image_edit(&upload())
    .await
    .expect("the edit succeeds");
    assert_eq!(answer, response());
    let recorded = provider.take();
    let [sent] = recorded.as_slice() else {
        panic!("exactly one send");
    };
    assert_eq!(sent.model, "gpt-image-1");
    assert_eq!(sent.prompt.as_deref(), Some("change"));
    assert_eq!(
        sent.images
            .iter()
            .map(|file| file.filename.as_deref())
            .collect::<Vec<_>>(),
        [Some("a.png"), Some("b.png")]
    );
    assert_eq!(
        sent.mask.as_ref().and_then(|file| file.filename.as_deref()),
        Some("m.png")
    );
    assert_eq!(
        sent.optional_params,
        ImageParams::from([
            (String::from("mask"), Json::Text(String::from("m.png"))),
            (String::from("n"), Json::Number(Number::from(1))),
            (String::from("quality"), Json::Text(String::from("high"))),
            (
                String::from("response_format"),
                Json::Text(String::from("b64_json"))
            ),
            (String::from("size"), Json::Text(String::from("1024x1024"))),
            (String::from("user"), Json::Text(String::from("u"))),
        ])
    );
    assert_eq!(sent.custom_llm_provider, "openai");
    assert_eq!(sent.timeout, Some(6000.0));
    assert!(!sent.asynchronous);
    let [record] = logger.records.as_slice() else {
        panic!("exactly one logged pre-call");
    };
    assert_eq!(record.model, "gpt-image-1");
    assert_eq!(record.user.as_deref(), Some("u"));
    assert_eq!(record.optional_params, sent.optional_params);
    assert_eq!(
        record.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert_eq!(record.litellm_params.get("model_info"), Some(&Json::Null));
}

#[tokio::test]
async fn edit_openai_upload_normalizes_images_and_defaults_the_model() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&[], true));
    let source = Source::new(vec![
        (gpt_image_tag(), Arc::clone(&provider)),
        (
            ImageEditConfigTag::OpenAi(String::from("dall-e-2")),
            Arc::clone(&provider),
        ),
    ]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut logger = RecordingLogger::new();
    let mut single = upload();
    single.image = ImageInput::One(Box::new(image("a.png", b"abc")));
    single.mask = None;
    single.model = None;
    single.count = None;
    single.quality = None;
    single.response_format = None;
    single.size = None;
    single.user = None;
    single.prompt = Some(String::from("single"));
    ImageEditEntry {
        operation: &service,
        logger: &mut logger,
        redactor: &redactor(),
    }
    .image_edit(&single)
    .await
    .expect("the single-image edit succeeds");
    let mut absent = single.clone();
    absent.image = ImageInput::Absent;
    absent.prompt = Some(String::from("none"));
    ImageEditEntry {
        operation: &service,
        logger: &mut logger,
        redactor: &redactor(),
    }
    .image_edit(&absent)
    .await
    .expect("the image-less edit succeeds");
    let recorded = provider.take();
    let [single_sent, absent_sent] = recorded.as_slice() else {
        panic!("one send per edit");
    };
    assert_eq!(single_sent.model, "dall-e-2");
    assert_eq!(single_sent.images.len(), 1);
    assert_eq!(single_sent.images[0].filename.as_deref(), Some("a.png"));
    assert_eq!(absent_sent.model, "dall-e-2");
    assert_eq!(absent_sent.images.len(), 0);
    assert!(absent_sent.optional_params.is_empty());
}

#[tokio::test]
async fn edit_option_merge() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(
        &["image", "prompt", "mask", "model", "n"],
        true,
    ));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut logger = RecordingLogger::new();
    let mut call = upload();
    call.image = ImageInput::One(Box::new(image("a.png", b"abc")));
    call.mask = None;
    call.prompt = Some(String::from("edit"));
    call.quality = None;
    call.response_format = None;
    call.size = None;
    call.user = None;
    call.extra_headers = Some(ImageParams::from([(
        String::from("x"),
        Json::Text(String::from("new")),
    )]));
    call.extra_query = Some(ImageParams::from([(
        String::from("ignored"),
        Json::Text(String::from("yes")),
    )]));
    call.extra_body = Some(ImageParams::from([
        (
            String::from("settings"),
            Json::Object(IndexMap::from([(
                String::from("strength"),
                Json::Number(Number::from(2)),
            )])),
        ),
        (
            String::from("tags"),
            Json::List(vec![
                Json::Text(String::from("a")),
                Json::Text(String::from("b")),
            ]),
        ),
    ]));
    call.kwargs = ImageParams::from([
        (
            String::from("headers"),
            Json::Object(IndexMap::from([(
                String::from("x"),
                Json::Text(String::from("old")),
            )])),
        ),
        (
            String::from("settings"),
            Json::Object(IndexMap::from([(
                String::from("strength"),
                Json::Number(Number::from(1)),
            )])),
        ),
        (String::from("api_key"), Json::Text(String::from("sk-test"))),
    ]);
    ImageEditEntry {
        operation: &service,
        logger: &mut logger,
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the merged edit succeeds");
    let recorded = provider.take();
    let [sent] = recorded.as_slice() else {
        panic!("exactly one send");
    };
    assert_eq!(
        sent.extra_headers,
        Some(ImageHeaders::from([(
            String::from("x"),
            String::from("new")
        )]))
    );
    assert_eq!(sent.api_key.as_deref(), Some("sk-test"));
    assert_eq!(
        sent.optional_params,
        ImageParams::from([
            (String::from("n"), Json::Number(Number::from(1))),
            (
                String::from("settings[strength]"),
                Json::Text(String::from("2"))
            ),
            (
                String::from("tags"),
                Json::List(vec![
                    Json::Text(String::from("a")),
                    Json::Text(String::from("b"))
                ])
            ),
        ])
    );
    assert!(sent.extra_body.is_some());
    let [record] = logger.records.as_slice() else {
        panic!("exactly one logged pre-call");
    };
    assert_eq!(record.optional_params, sent.optional_params);
}

#[tokio::test]
async fn edit_option_merge_json_provider() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&["n", "size", "seed"], false));
    let source = Source::new(vec![(
        ImageEditConfigTag::Gemini(String::from("edit-model")),
        Arc::clone(&provider),
    )]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut logger = RecordingLogger::new();
    let mut call = upload();
    call.image = ImageInput::One(Box::new(image("a.png", b"abc")));
    call.mask = None;
    call.model = Some(String::from("gemini/edit-model"));
    call.prompt = Some(String::from("edit"));
    call.quality = None;
    call.response_format = None;
    call.user = None;
    call.size = Some(String::from("1024x1024"));
    let body = ImageParams::from([
        (
            String::from("settings"),
            Json::Object(IndexMap::from([(
                String::from("strength"),
                Json::Number(Number::from(2)),
            )])),
        ),
        (
            String::from("tags"),
            Json::List(vec![
                Json::Text(String::from("a")),
                Json::Text(String::from("b")),
            ]),
        ),
    ]);
    call.extra_body = Some(body.clone());
    call.kwargs = ImageParams::from([
        (String::from("seed"), Json::Number(Number::from(7))),
        (
            String::from("settings"),
            Json::Object(IndexMap::from([(
                String::from("strength"),
                Json::Number(Number::from(1)),
            )])),
        ),
    ]);
    ImageEditEntry {
        operation: &service,
        logger: &mut logger,
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the JSON edit succeeds");
    let recorded = provider.take();
    let [sent] = recorded.as_slice() else {
        panic!("exactly one send");
    };
    assert_eq!(
        sent.optional_params,
        ImageParams::from([
            (String::from("n"), Json::Number(Number::from(1))),
            (String::from("size"), Json::Text(String::from("1024x1024"))),
            (String::from("seed"), Json::Number(Number::from(7))),
        ])
    );
    assert_eq!(sent.extra_body.as_ref(), Some(&body));
    assert_eq!(sent.custom_llm_provider, "gemini");
}

#[tokio::test]
async fn edit_option_merge_rejects_a_non_text_header() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&[], false));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.extra_headers = Some(ImageParams::from([(
        String::from("x"),
        Json::Number(Number::from(1)),
    )]));
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect_err("a numeric header value is refused");
    assert_eq!(error.message, "the x header value must be text");
    assert_eq!(provider.take().len(), 0);
}

#[rstest]
#[case::stability_none("stability/edit-model", None, Some(7.0))]
#[case::stability_zero("stability/edit-model", Some(0.0), Some(7.0))]
#[case::stability_explicit("stability/edit-model", Some(3.0), Some(3.0))]
#[case::bedrock_none("bedrock/edit-model", None, None)]
#[case::bedrock_zero("bedrock/edit-model", Some(0.0), Some(0.0))]
#[case::bedrock_explicit("bedrock/edit-model", Some(3.0), Some(3.0))]
#[case::black_forest_labs_none("black_forest_labs/edit-model", None, Some(7.0))]
#[tokio::test]
async fn edit_dispatch_and_timeout(
    #[case] model: &str,
    #[case] timeout: Option<f64>,
    #[case] expected: Option<f64>,
) {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[("REQUEST_TIMEOUT", "7")]);
    let stability = Arc::new(Provider::new(&[], false));
    let bedrock = Arc::new(Provider::new(&[], false));
    let bfl = Arc::new(Provider::new(&[], false));
    let source = Source::new(vec![
        (
            ImageEditConfigTag::Stability(String::from("edit-model")),
            Arc::clone(&stability),
        ),
        (
            ImageEditConfigTag::Bedrock(String::from("edit-model")),
            Arc::clone(&bedrock),
        ),
        (ImageEditConfigTag::BlackForestLabs, Arc::clone(&bfl)),
    ]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.model = Some(model.to_owned());
    call.image = ImageInput::One(Box::new(image("a.png", b"abc")));
    call.mask = None;
    call.timeout = timeout;
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the fixture branch answers");
    let provider = match model {
        "stability/edit-model" => &stability,
        "bedrock/edit-model" => &bedrock,
        _ => &bfl,
    };
    let timeouts: Vec<Option<f64>> = provider.take().iter().map(|sent| sent.timeout).collect();
    assert_eq!(timeouts, [expected]);
}

#[tokio::test]
async fn edit_dispatch_and_timeout_defaults_to_6000() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let stability = Arc::new(Provider::new(&[], false));
    let source = Source::new(vec![(
        ImageEditConfigTag::Stability(String::from("edit-model")),
        Arc::clone(&stability),
    )]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.model = Some(String::from("stability/edit-model"));
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the fixture branch answers");
    assert_eq!(stability.take()[0].timeout, Some(6000.0));
}

#[tokio::test]
async fn edit_dispatch_and_timeout_without_a_config() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let source = Source::new(Vec::new());
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.model = Some(String::from("anthropic/claude-3-5-haiku-20241022"));
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect_err("anthropic has no image edit config");
    assert_eq!(error.message, "image edit is not supported for anthropic");
    assert!(matches!(error.class, ErrorClass::ApiConnection(_)));
    let metadata = error.class.metadata();
    assert_eq!(metadata.model.as_deref(), Some("claude-3-5-haiku-20241022"));
    assert_eq!(metadata.provider.as_deref(), Some("anthropic"));
    let exception = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .aimage_edit(&call)
    .await
    .expect_err("the async entry maps the same failure");
    assert_eq!(
        exception.message,
        "image edit is not supported for anthropic"
    );
    assert!(matches!(exception.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn edit_dispatch_and_timeout_logs_before_the_branch_merge() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let stability = Arc::new(Provider::new(&["n", "size"], false));
    let source = Source::new(vec![(
        ImageEditConfigTag::Stability(String::from("edit-model")),
        Arc::clone(&stability),
    )]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.model = Some(String::from("stability/edit-model"));
    call.image = ImageInput::One(Box::new(image("a.png", b"abc")));
    call.mask = None;
    call.kwargs = ImageParams::from([(
        String::from("settings"),
        Json::Object(IndexMap::from([(
            String::from("strength"),
            Json::Number(Number::from(1)),
        )])),
    )]);
    let mut logger = RecordingLogger::new();
    ImageEditEntry {
        operation: &service,
        logger: &mut logger,
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the fixture branch answers");
    let [record] = logger.records.as_slice() else {
        panic!("exactly one logged pre-call");
    };
    assert!(record.optional_params.is_empty());
    let recorded = stability.take();
    let [sent] = recorded.as_slice() else {
        panic!("exactly one send");
    };
    assert_eq!(
        sent.optional_params,
        ImageParams::from([(
            String::from("settings"),
            Json::Object(IndexMap::from([(
                String::from("strength"),
                Json::Number(Number::from(1)),
            )]))
        )])
    );
}

#[tokio::test]
async fn edit_custom_and_async() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let first = Arc::new(CustomHandler::new("first"));
    let second = Arc::new(CustomHandler::new("second"));
    let custom = CustomSource::new(vec![(
        "my-provider",
        vec![Arc::clone(&first), Arc::clone(&second)],
    )]);
    let source = Source::new(Vec::new());
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &custom,
        global_drop_params: false,
    };
    let mut call = custom_call();
    call.kwargs
        .insert(String::from("async_call"), Json::Number(Number::from(1)));
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the custom handler answers");
    assert!(first.take().is_empty());
    let calls = second.take();
    let [sent] = calls.as_slice() else {
        panic!("exactly one custom call");
    };
    assert!(!sent.asynchronous);
    assert_eq!(sent.model, "my-model");
    assert_eq!(
        sent.images
            .iter()
            .map(|image| (image.filename.as_deref(), image.content.as_slice()))
            .collect::<Vec<_>>(),
        [
            (Some("a.png"), b"abc".as_slice()),
            (Some("b.png"), b"def".as_slice())
        ]
    );
    assert_eq!(
        sent.optional_params.get("settings"),
        call.kwargs.get("settings")
    );
    assert!(!sent.optional_params.contains_key("async_call"));
    assert_eq!(sent.api_key.as_deref(), Some("sk-test"));
    assert_eq!(sent.client, Some(ImageEditClientKind::Sync));
    let mut wrong = call.clone();
    wrong.client = Some(ImageEditClientKind::Asynchronous);
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&wrong)
    .await
    .expect("the custom handler answers");
    assert_eq!(second.take()[0].client, None);
}

#[tokio::test]
async fn edit_custom_and_async_forces_async() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let handler = Arc::new(CustomHandler::new("async").pending());
    let custom = CustomSource::new(vec![("my-provider", vec![Arc::clone(&handler)])]);
    let source = Source::new(Vec::new());
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &custom,
        global_drop_params: false,
    };
    let mut call = custom_call();
    call.image = ImageInput::One(Box::new(image("a.png", b"abc")));
    call.client = Some(ImageEditClientKind::Asynchronous);
    let answer = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .aimage_edit(&call)
    .await
    .expect("the async handler answers");
    assert_eq!(answer, response());
    assert_eq!(call.kwargs.get("async_call"), None);
    let calls = handler.take();
    let [sent] = calls.as_slice() else {
        panic!("exactly one custom call");
    };
    assert!(sent.asynchronous);
    assert_eq!(sent.client, Some(ImageEditClientKind::Asynchronous));
    assert_eq!(sent.images.len(), 1);
}

#[tokio::test]
async fn edit_custom_and_async_missing_handler() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let custom = CustomSource::new(vec![("ghost-provider", Vec::new())]);
    let source = Source::new(Vec::new());
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &custom,
        global_drop_params: false,
    };
    let mut call = custom_call();
    call.model = Some(String::from("ghost-model"));
    call.custom_llm_provider = Some(String::from("ghost-provider"));
    call.image = ImageInput::Absent;
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect_err("a listed provider without a handler is unknown");
    let expected = "Unmapped LLM provider for this endpoint. You passed model=ghost-model, custom_llm_provider=ghost-provider. Check supported provider and route: https://docs.litellm.ai/docs/providers";
    assert_eq!(error.message, expected);
    assert!(matches!(error.class, ErrorClass::BadRequest(_)));
    let exception = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .aimage_edit(&call)
    .await
    .expect_err("the async entry maps the same failure");
    assert_eq!(exception.message, expected);
    assert!(matches!(exception.class, ErrorClass::BadRequest(_)));
}

#[tokio::test]
async fn image_edit_awaits_a_pending_provider() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&[], false).pending());
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let answer = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&upload())
    .await
    .expect("the pending provider answers");
    assert_eq!(answer, response());
    assert_eq!(provider.take().len(), 1);
}

#[tokio::test]
async fn image_edit_reports_a_logger_failure() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&[], false));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut FailingLogger,
        redactor: &redactor(),
    }
    .image_edit(&upload())
    .await
    .expect_err("the logger's refusal is reported");
    assert_eq!(
        error.message,
        ImageEditError::Logging(PrePostError::RawRequestCaptured).to_string()
    );
    assert_eq!(provider.take().len(), 0);
}

#[test]
fn image_edit_dispatch_defaults() {
    assert_eq!(DEFAULT_MODEL, "dall-e-2");
    assert_eq!(DEFAULT_REQUEST_TIMEOUT, 6000.0);
    let settings = settings(&[]);
    assert_eq!(settings.request_timeout_seconds, 6000.0);
    assert_eq!(
        branch_timeout(
            &ImageEditConfigTag::Bedrock(String::from("m")),
            Some(0.0),
            7.0
        )
        .expect("bedrock keeps zero"),
        Some(0.0)
    );
    assert_eq!(
        branch_timeout(
            &ImageEditConfigTag::Stability(String::from("m")),
            Some(0.0),
            7.0
        )
        .expect("stability falls back"),
        Some(7.0)
    );
    let refused = branch_timeout(
        &ImageEditConfigTag::Bedrock(String::from("m")),
        Some(-1.0),
        7.0,
    )
    .expect_err("a negative timeout is refused");
    assert_eq!(
        refused.to_string(),
        "timeout must be a non-negative number of seconds, got -1"
    );
}

#[tokio::test]
async fn edit_option_merge_passes_the_drop_policy() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&[], true));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: true,
    };
    let mut call = upload();
    call.kwargs = ImageParams::from([
        (String::from("drop_params"), Json::Bool(true)),
        (
            String::from("additional_drop_params"),
            Json::List(vec![
                Json::Text(String::from("size")),
                Json::Number(Number::from(1)),
            ]),
        ),
    ]);
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the edit succeeds");
    assert_eq!(
        provider.policies(),
        [OptionalParamsPolicy {
            global_drop_params: true,
            call_drop_params: true,
            additional_drop_params: vec![String::from("size")],
        }]
    );
}

#[rstest]
#[case::openai_without_multipart(gpt_image_tag(), "openai/gpt-image-1", false)]
#[case::gemini_with_multipart(
    ImageEditConfigTag::Gemini(String::from("edit-model")),
    "gemini/edit-model",
    true
)]
#[tokio::test]
async fn edit_option_merge_form_fields_need_an_openai_multipart_config(
    #[case] tag: ImageEditConfigTag,
    #[case] model: &str,
    #[case] multipart: bool,
) {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&[], multipart));
    let source = Source::new(vec![(tag, Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.model = Some(model.to_owned());
    call.extra_body = Some(ImageParams::from([(
        String::from("tags"),
        Json::List(vec![Json::Text(String::from("a"))]),
    )]));
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("the edit succeeds");
    let recorded = provider.take();
    let [sent] = recorded.as_slice() else {
        panic!("exactly one send");
    };
    assert!(!sent.optional_params.contains_key("tags"));
}

#[tokio::test]
async fn image_edit_attributes_an_unresolved_model_to_the_arguments() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let source = Source::new(Vec::new());
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.model = Some(String::from("unlisted-edit-model"));
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect_err("an unknown model has no provider");
    assert!(matches!(error.class, ErrorClass::BadRequest(_)));
    let metadata = error.class.metadata();
    assert_eq!(metadata.model.as_deref(), Some("unlisted-edit-model"));
    assert_eq!(metadata.status_code, 400);
}
