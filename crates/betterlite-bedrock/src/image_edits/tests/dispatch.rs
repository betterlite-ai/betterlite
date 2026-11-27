#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use serde_json::Number;

use super::cases::{
    Answer, CANVAS_COST, CANVAS_MODEL, STABILITY_COST, STABILITY_MODEL, STYLE_MODEL, answered,
    auth_params, call, image_list, run,
};
use super::fixture::Reply;
use crate::canvas_edits::error::CanvasEditError;
use crate::image_edits::error::ImageEditError;
use crate::image_edits::plan::EditImage;

pub(super) fn canvas_edit_dispatch(redactor: &Redactor) {
    canvas_variation(redactor);
    canvas_first_image_only(redactor);
    canvas_background_removal(redactor);
}

fn canvas_variation(redactor: &Redactor) {
    // the raw model_cost flag picks the Nova Canvas config, and its body names the task first
    let supplied = image_list(&["aW1n"]);
    let mut options = auth_params();
    options.insert(
        "numberOfImages".to_owned(),
        Json::Number(Number::from(2_i64)),
    );
    options.insert("width".to_owned(), Json::Number(Number::from(512_i64)));
    let mut canvas = call(CANVAS_MODEL, Some("edit it"), options);
    canvas.images = &supplied;
    let variation = run(
        canvas,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["b3V0"]}"#)),
        redactor,
    );
    assert_eq!(variation.images(), ["b3V0"]);
    assert_eq!(variation.cost().as_deref(), Some("0.25"));
    assert_eq!(
        variation.text(),
        r#"{"taskType": "IMAGE_VARIATION", "imageVariationParams": {"images": ["aW1n"], "text": "edit it"}, "imageGenerationConfig": {"width": 512, "numberOfImages": 2}}"#
    );
}

fn canvas_first_image_only(redactor: &Redactor) {
    // a second image never reaches the transform: LiteLLM hands it image[0] alone
    let extra = image_list(&["aW1n", "dHdv"]);
    let mut many = call(CANVAS_MODEL, Some("edit it"), auth_params());
    many.images = &extra;
    let first_only = run(
        many,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["b3V0"]}"#)),
        redactor,
    );
    assert_eq!(
        first_only.text(),
        r#"{"taskType": "IMAGE_VARIATION", "imageVariationParams": {"images": ["aW1n"], "text": "edit it"}}"#
    );
}

fn canvas_background_removal(redactor: &Redactor) {
    // BACKGROUND_REMOVAL keeps only its own params record, even with a generation option set
    let supplied = image_list(&["aW1n"]);
    let mut removal_options = auth_params();
    removal_options.insert(
        "taskType".to_owned(),
        Json::Text("BACKGROUND_REMOVAL".to_owned()),
    );
    removal_options.insert(
        "numberOfImages".to_owned(),
        Json::Number(Number::from(2_i64)),
    );
    let mut removal = call(CANVAS_MODEL, None, removal_options);
    removal.images = &supplied;
    let removed = run(
        removal,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["YmFjaw=="]}"#)),
        redactor,
    );
    assert_eq!(
        removed.text(),
        r#"{"taskType": "BACKGROUND_REMOVAL", "backgroundRemovalParams": {"image": "aW1n"}}"#
    );
}

pub(super) fn stability_edit_dispatch(redactor: &Redactor) {
    stability_named_edit(redactor);
    stability_style_transfer(redactor);
    stability_promptless(redactor);
    stability_imageless(redactor);
}

fn stability_named_edit(redactor: &Redactor) {
    // the name markers pick Stability, and the body fixes the png output format before the prompt
    let supplied = image_list(&["aW1n"]);
    let mut options = auth_params();
    options.insert("aspect_ratio".to_owned(), Json::Text("16:9".to_owned()));
    options.insert("seed".to_owned(), Json::Number(Number::from(7_i64)));
    options.insert("quality".to_owned(), Json::Text("standard".to_owned()));
    let mut stability = call(STABILITY_MODEL, Some("edit it"), options);
    stability.images = &supplied;
    let edited = run(
        stability,
        STABILITY_COST,
        answered(Reply::ok(r#"{"images": ["c3RhYg=="]}"#)),
        redactor,
    );
    assert_eq!(edited.images(), ["c3RhYg=="]);
    assert_eq!(edited.cost().as_deref(), Some("0.07"));
    assert_eq!(
        edited.text(),
        r#"{"output_format": "png", "prompt": "edit it", "image": "aW1n", "aspect_ratio": "16:9", "seed": 7}"#
    );
}

fn stability_style_transfer(redactor: &Redactor) {
    // a style-transfer model names its source image init_image
    let supplied = image_list(&["aW1n"]);
    let mut transfer = call(STYLE_MODEL, Some("edit it"), auth_params());
    transfer.images = &supplied;
    let styled = run(
        transfer,
        "{}",
        answered(Reply::ok(r#"{"images": ["c3R5bGU="]}"#)),
        redactor,
    );
    assert_eq!(
        styled.text(),
        r#"{"output_format": "png", "prompt": "edit it", "init_image": "aW1n"}"#
    );
    // an unmapped stability model still answers, at the zero price
    assert_eq!(styled.cost().as_deref(), Some("0.0"));
}

fn stability_promptless(redactor: &Redactor) {
    // without a prompt the key is absent, and pre_call records a null input
    let supplied = image_list(&["aW1n"]);
    let mut bare = call(STABILITY_MODEL, None, auth_params());
    bare.images = &supplied;
    let promptless = run(
        bare,
        STABILITY_COST,
        answered(Reply::ok(r#"{"images": ["cXVpZXQ="]}"#)),
        redactor,
    );
    assert_eq!(
        promptless.text(),
        r#"{"output_format": "png", "image": "aW1n"}"#
    );
    assert_eq!(
        promptless.state.model_call_details.get("input"),
        Some(&Json::Null)
    );
}

fn stability_imageless(redactor: &Redactor) {
    // Stability permits no image at all, and an empty image list is a successful result
    let no_images: &[EditImage] = &[];
    let mut imageless = call(STABILITY_MODEL, Some("edit it"), auth_params());
    imageless.images = no_images;
    let empty = run(
        imageless,
        STABILITY_COST,
        answered(Reply::ok(r#"{"images": []}"#)),
        redactor,
    );
    assert_eq!(
        empty.text(),
        r#"{"output_format": "png", "prompt": "edit it"}"#
    );
    assert!(empty.images().is_empty());
    assert_eq!(empty.cost().as_deref(), Some("0.07"));
}

pub(super) fn unknown_edit_model(redactor: &Redactor) {
    // without a stability marker and without the nova-canvas flag there is no config class to build
    for prices in [
        "{}",
        r#"{"unknown-fixture": {}}"#,
        r#"{"unknown-fixture": {"supports_nova_canvas_image_edit": "true"}}"#,
    ] {
        let unknown = run(
            call("unknown-fixture", None, auth_params()),
            prices,
            Answer {
                reply: Reply::ok("{}"),
                supplied: None,
            },
            redactor,
        );
        assert!(matches!(
            unknown.error(),
            ImageEditError::Canvas(CanvasEditError::UnsupportedModel { .. })
        ));
        assert_eq!(
            unknown.error().to_string(),
            "Unsupported Bedrock image-edit model: 'unknown-fixture'. Use a stability.* image-edit model id or add supports_nova_canvas_image_edit in model_prices for this id."
        );
        assert_eq!(unknown.error().status(), None);
        assert_eq!(unknown.sync_calls, 0);
        assert!(unknown.factory_requests.is_empty());
        assert!(unknown.supplied_requests.is_empty());
    }
}
