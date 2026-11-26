use std::collections::BTreeMap;

use super::support::{CANVAS_MODEL, IN_PAINT, catalog, config_for, model_cost};
use crate::bedrock_settings::ErrorHeaders;
use crate::canvas_edits::environment::{
    USE_MULTIPART_FORM_DATA, get_error_class, validate_environment,
};
use crate::canvas_edits::error::CanvasEditError;
use crate::canvas_edits::selection::ImageEditConfig;
use crate::stability_edits::environment::COMPLETE_URL;

pub(super) fn canvas_edit_contract() {
    environment_contract();
    error_class_contract();
    complete_url_contract();
}

fn default_headers() -> BTreeMap<String, String> {
    BTreeMap::from([("Content-Type".to_owned(), "application/json".to_owned())])
}

fn environment_contract() {
    let multipart = USE_MULTIPART_FORM_DATA;
    assert!(!multipart);
    assert_eq!(validate_environment(None), default_headers());
    assert_eq!(
        validate_environment(Some(BTreeMap::new())),
        default_headers()
    );
    let given = BTreeMap::from([
        ("Authorization".to_owned(), "Bearer x".to_owned()),
        ("content-type".to_owned(), "text/plain".to_owned()),
    ]);
    let expected = BTreeMap::from([
        ("Authorization".to_owned(), "Bearer x".to_owned()),
        ("content-type".to_owned(), "text/plain".to_owned()),
        ("Content-Type".to_owned(), "application/json".to_owned()),
    ]);
    assert_eq!(validate_environment(Some(given)), expected);
    let content_type = BTreeMap::from([(
        "Content-Type".to_owned(),
        "application/x-amz-json-1.1".to_owned(),
    )]);
    assert_eq!(
        validate_environment(Some(content_type.clone())),
        content_type
    );
}

fn error_class_contract() {
    let headers = vec![(
        "x-amzn-errortype".to_owned(),
        "ValidationException".to_owned(),
    )];
    let error = get_error_class("the model is sad".to_owned(), 400, headers.clone());
    assert_eq!(error.status(), Some(400));
    assert_eq!(error.to_string(), "bedrock error (400)");
    let CanvasEditError::Provider {
        status,
        error: detail,
    } = &error
    else {
        panic!("expected a provider error");
    };
    assert_eq!(*status, 400);
    assert_eq!(detail.error.status_code, 400);
    assert_eq!(detail.error.message, "the model is sad");
    assert_eq!(detail.error.headers.as_deref(), Some(headers.as_slice()));
    assert_eq!(detail.headers, Some(ErrorHeaders::Http(headers.clone())));
    assert_eq!(detail.response_headers, headers);
    let without_headers = get_error_class("no headers".to_owned(), 503, Vec::new());
    let CanvasEditError::Provider {
        status: status_code,
        error: no_header_error,
    } = &without_headers
    else {
        panic!("expected a provider error");
    };
    assert_eq!(*status_code, 503);
    assert_eq!(
        no_header_error.headers,
        Some(ErrorHeaders::Http(Vec::new()))
    );
}

fn complete_url_contract() {
    let url = ImageEditConfig::Stability
        .complete_url()
        .expect("the stability config answers its url");
    assert_eq!(url, COMPLETE_URL);
    let error = ImageEditConfig::NovaCanvas
        .complete_url()
        .expect_err("the nova canvas config has no url");
    assert!(matches!(&error, CanvasEditError::CompleteUrl));
    assert_eq!(
        error.to_string(),
        "Nova Canvas image edit URLs are built in BedrockImageEdit._prepare_request (AWS runtime endpoint + model invoke path). Do not use get_complete_url for this config."
    );
    assert_eq!(error.status(), None);
}

pub(super) fn canvas_edit_selector() {
    let cost = catalog();
    let in_paint = config_for(&cost, IN_PAINT).expect("the inpaint model routes to stability");
    assert_eq!(in_paint, ImageEditConfig::Stability);
    let upscale = config_for(&model_cost("{}"), "stability.stable-image-upscale-1:0")
        .expect("stability detection needs no catalog");
    assert_eq!(upscale, ImageEditConfig::Stability);
    for model in [
        CANVAS_MODEL,
        "CANVAS-FIXTURE-PLAIN",
        "x/canvas-fixture-suffix",
    ] {
        let routed = config_for(&cost, model).expect("the catalog flag routes to nova canvas");
        assert_eq!(routed, ImageEditConfig::NovaCanvas, "{model}");
    }
    let error =
        config_for(&cost, "amazon.nova-canvas-v1:0").expect_err("an unmarked model is refused");
    assert!(
        matches!(&error, CanvasEditError::UnsupportedModel { model } if model == "amazon.nova-canvas-v1:0")
    );
    assert_eq!(
        error.to_string(),
        "Unsupported Bedrock image-edit model: 'amazon.nova-canvas-v1:0'. Use a stability.* image-edit model id or add supports_nova_canvas_image_edit in model_prices for this id."
    );
    assert_eq!(error.status(), None);
    let empty_model_error =
        config_for(&model_cost("{}"), "").expect_err("an empty model is refused");
    assert_eq!(
        empty_model_error.to_string(),
        "Unsupported Bedrock image-edit model: ''. Use a stability.* image-edit model id or add supports_nova_canvas_image_edit in model_prices for this id."
    );
    let unmarked_error =
        config_for(&model_cost("{}"), CANVAS_MODEL).expect_err("a spelling alone is not enough");
    assert!(
        matches!(&unmarked_error, CanvasEditError::UnsupportedModel { model } if model == CANVAS_MODEL)
    );
}
