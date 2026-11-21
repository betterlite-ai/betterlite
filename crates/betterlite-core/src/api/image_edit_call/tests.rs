#![cfg(test)]
#![expect(
    clippy::too_many_lines,
    reason = "one test per documented behavior, each asserting the whole recorded call"
)]

use std::sync::Arc;

use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use serde_json::Number;

use self::support::{
    CustomHandler, CustomSource, FailingLogger, NoCustom, Provider, RecordingLogger, Source, World,
    custom_call, gpt_image_tag, image, redactor, response, settings, upload,
};
use super::call::{DEFAULT_MODEL, DEFAULT_REQUEST_TIMEOUT, ImageEditClientKind, ImageInput};
use super::entry::{aimage_edit, image_edit};
use super::error::ImageEditError;
use super::service::{ImageEdits, branch_timeout, check_branch_model};
use crate::api::image_input_options::options::{Quality, ResponseFormat};

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
    let answer = image_edit(&service, &upload(), &mut logger)
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
    image_edit(&service, &single, &mut logger)
        .await
        .expect("the single-image edit succeeds");
    let mut absent = single.clone();
    absent.image = ImageInput::Absent;
    absent.prompt = Some(String::from("none"));
    image_edit(&service, &absent, &mut logger)
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
    image_edit(&service, &call, &mut logger)
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
    image_edit(&service, &call, &mut logger)
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
    let error = image_edit(&service, &call, &mut RecordingLogger::new())
        .await
        .expect_err("a numeric header value is refused");
    assert_eq!(error.to_string(), "the x header value must be text");
    assert_eq!(provider.take().len(), 0);
}

#[tokio::test]
async fn edit_dispatch_and_timeout() {
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
    for (model, timeout) in [
        ("stability/edit-model", None),
        ("stability/edit-model", Some(0.0)),
        ("stability/edit-model", Some(3.0)),
        ("bedrock/edit-model", None),
        ("bedrock/edit-model", Some(0.0)),
        ("bedrock/edit-model", Some(3.0)),
        ("black_forest_labs/edit-model", None),
    ] {
        let mut call = upload();
        call.model = Some(model.to_owned());
        call.image = ImageInput::One(Box::new(image("a.png", b"abc")));
        call.mask = None;
        call.timeout = timeout;
        image_edit(&service, &call, &mut RecordingLogger::new())
            .await
            .expect("the fixture branch answers");
    }
    let stability_timeouts: Vec<Option<f64>> =
        stability.take().iter().map(|sent| sent.timeout).collect();
    assert_eq!(stability_timeouts, [Some(7.0), Some(7.0), Some(3.0)]);
    let bedrock_timeouts: Vec<Option<f64>> =
        bedrock.take().iter().map(|sent| sent.timeout).collect();
    assert_eq!(bedrock_timeouts, [None, Some(0.0), Some(3.0)]);
    let bfl_timeouts: Vec<Option<f64>> = bfl.take().iter().map(|sent| sent.timeout).collect();
    assert_eq!(bfl_timeouts, [Some(7.0)]);
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
    image_edit(&service, &call, &mut RecordingLogger::new())
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
    let error = image_edit(&service, &call, &mut RecordingLogger::new())
        .await
        .expect_err("anthropic has no image edit config");
    assert_eq!(
        error.to_string(),
        "image edit is not supported for anthropic"
    );
    let exception = aimage_edit(&service, &call, &mut RecordingLogger::new(), &redactor())
        .await
        .expect_err("the async entry maps the same failure");
    assert_eq!(
        exception.message,
        "image edit is not supported for anthropic"
    );
    assert!(matches!(exception.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn edit_dispatch_and_timeout_requires_a_model() {
    assert_eq!(
        check_branch_model(&ImageEditConfigTag::Bedrock(String::from("m")), None)
            .expect_err("bedrock needs a model")
            .to_string(),
        "Model needs to be set for bedrock"
    );
    assert_eq!(
        check_branch_model(&ImageEditConfigTag::BlackForestLabs, None)
            .expect_err("black forest labs needs a model")
            .to_string(),
        "Model needs to be set for black_forest_labs"
    );
    check_branch_model(&ImageEditConfigTag::Stability(String::from("m")), None)
        .expect("stability accepts no model");
    check_branch_model(&ImageEditConfigTag::Bedrock(String::from("m")), Some("m"))
        .expect("bedrock accepts a model");
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
    image_edit(&service, &call, &mut logger)
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
    image_edit(&service, &call, &mut RecordingLogger::new())
        .await
        .expect("the custom handler answers");
    assert!(first.take().is_empty());
    let calls = second.take();
    let [sent] = calls.as_slice() else {
        panic!("exactly one custom call");
    };
    assert_eq!(sent.handler, "second");
    assert!(!sent.asynchronous);
    assert_eq!(sent.model, "my-model");
    assert_eq!(
        sent.images
            .iter()
            .map(|(name, content)| (name.as_deref(), content.as_slice()))
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
    image_edit(&service, &wrong, &mut RecordingLogger::new())
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
    let answer = aimage_edit(&service, &call, &mut RecordingLogger::new(), &redactor())
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
    let error = image_edit(&service, &call, &mut RecordingLogger::new())
        .await
        .expect_err("a listed provider without a handler is unknown");
    let expected = "Unmapped LLM provider for this endpoint. You passed model=ghost-model, custom_llm_provider=ghost-provider. Check supported provider and route: https://docs.litellm.ai/docs/providers";
    assert_eq!(error.to_string(), expected);
    let exception = aimage_edit(&service, &call, &mut RecordingLogger::new(), &redactor())
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
    let answer = image_edit(&service, &upload(), &mut RecordingLogger::new())
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
    let error = image_edit(&service, &upload(), &mut FailingLogger)
        .await
        .expect_err("the logger's refusal is reported");
    assert!(matches!(error, ImageEditError::Logging(_)));
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
}
