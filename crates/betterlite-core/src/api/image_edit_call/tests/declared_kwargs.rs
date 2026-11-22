use std::sync::Arc;

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use indexmap::IndexMap;
use serde_json::Number;

use super::UPLOAD_OPTIONS;
use super::support::{
    NoCustom, Provider, RecordingLogger, Source, World, gpt_image_tag, redactor, settings, upload,
};
use crate::api::image_edit_call::entry::ImageEditEntry;
use crate::api::image_edit_call::service::ImageEdits;

#[tokio::test]
async fn edit_option_merge_takes_declared_kwargs() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let supported = ["background", "input_fidelity", "imageConfig"];
    let provider = Arc::new(Provider::new(&supported, false));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.mask = None;
    call.count = None;
    call.quality = None;
    call.response_format = None;
    call.size = None;
    call.user = None;
    call.kwargs = ImageParams::from([
        (
            String::from("background"),
            Json::Text(String::from("opaque")),
        ),
        (
            String::from("input_fidelity"),
            Json::Text(String::from("high")),
        ),
        (
            String::from("imageConfig"),
            Json::Object(IndexMap::from([(
                String::from("seed"),
                Json::Number(Number::from(7_u32)),
            )])),
        ),
    ]);
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("a provider that advertises the declared kwargs takes them");
    let recorded = provider.take();
    let [sent] = recorded.as_slice() else {
        panic!("exactly one send");
    };
    assert_eq!(
        sent.optional_params,
        ImageParams::from([
            (
                String::from("background"),
                Json::Text(String::from("opaque"))
            ),
            (
                String::from("input_fidelity"),
                Json::Text(String::from("high"))
            ),
            (
                String::from("imageConfig"),
                Json::Object(IndexMap::from([(
                    String::from("seed"),
                    Json::Number(Number::from(7_u32)),
                )])),
            ),
        ])
    );
}

#[tokio::test]
async fn edit_option_merge_rejects_an_unsupported_background() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let supported = [
        "mask",
        "n",
        "quality",
        "response_format",
        "size",
        "user",
        "input_fidelity",
    ];
    let provider = Arc::new(Provider::new(&supported, true));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.kwargs = ImageParams::from([(
        String::from("background"),
        Json::Text(String::from("transparent")),
    )]);
    let mut logger = RecordingLogger::new();
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut logger,
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect_err("a provider that does not advertise background refuses it");
    assert_eq!(
        error.message,
        "The following parameters are not supported for model gpt-image-1: background"
    );
    assert!(matches!(error.class, ErrorClass::UnsupportedParams(_)));
    let metadata = error.class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert!(logger.records.is_empty());
    assert!(provider.take().is_empty());
}

#[tokio::test]
async fn edit_option_merge_drops_a_null_background() {
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
        global_drop_params: false,
    };
    let mut call = upload();
    call.mask = None;
    call.count = None;
    call.quality = None;
    call.response_format = None;
    call.size = None;
    call.user = None;
    call.kwargs = ImageParams::from([(String::from("background"), Json::Null)]);
    ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect("a null background is absent, not unsupported");
    let recorded = provider.take();
    let [sent] = recorded.as_slice() else {
        panic!("exactly one send");
    };
    assert!(
        sent.optional_params.is_empty(),
        "a null background is absent, so neither the options nor the form fields carry it"
    );
}

#[tokio::test]
async fn edit_option_merge_names_a_background_outside_its_literal() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&["background"], false));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.mask = None;
    call.count = None;
    call.quality = None;
    call.response_format = None;
    call.size = None;
    call.user = None;
    call.kwargs = ImageParams::from([(
        String::from("background"),
        Json::Text(String::from("purple")),
    )]);
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect_err("purple is not one of background's literals");
    assert!(matches!(error.class, ErrorClass::BadRequest(_)));
    assert_eq!(error.class.metadata().status_code, 400);
    assert!(
        error.message.contains("background"),
        "a value outside the declared literal names the field: {}",
        error.message
    );
    assert!(provider.take().is_empty());
}

#[tokio::test]
async fn edit_option_merge_rejects_unsupported_declared_kwargs() {
    let world = World::new();
    let context = world.context();
    let settings = settings(&[]);
    let provider = Arc::new(Provider::new(&UPLOAD_OPTIONS, true));
    let source = Source::new(vec![(gpt_image_tag(), Arc::clone(&provider))]);
    let service = ImageEdits {
        settings: &settings,
        resolution: &context,
        providers: &source,
        custom: &NoCustom,
        global_drop_params: false,
    };
    let mut call = upload();
    call.kwargs = ImageParams::from([
        (
            String::from("background"),
            Json::Text(String::from("opaque")),
        ),
        (
            String::from("input_fidelity"),
            Json::Text(String::from("high")),
        ),
        (
            String::from("imageConfig"),
            Json::Object(IndexMap::from([(
                String::from("seed"),
                Json::Number(Number::from(7_u32)),
            )])),
        ),
    ]);
    let error = ImageEditEntry {
        operation: &service,
        logger: &mut RecordingLogger::new(),
        redactor: &redactor(),
    }
    .image_edit(&call)
    .await
    .expect_err("a provider that supports none of the declared kwargs refuses them");
    assert_eq!(
        error.message,
        "The following parameters are not supported for model gpt-image-1: background, input_fidelity, imageConfig"
    );
    assert!(matches!(error.class, ErrorClass::UnsupportedParams(_)));
    assert!(provider.take().is_empty());
}
