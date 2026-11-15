#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_openai::image_edits::request::ImageEditForm;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::media::{MaskInput, SuppliedFile, SuppliedMedia};
use super::{
    EditError, MULTIPART_FORM_DATA, SUPPORTED_OPENAI_PARAMS, async_transform_image_edit_request,
    complete_url, edit_endpoint, map_openai_params, transform_image_edit_request,
    transform_image_edit_response,
};
use crate::api::bfl::common::BflError;

const NOW: i64 = 1_700_000_000;
const DEPTH: i64 = 100;

fn nested(levels: usize) -> SuppliedMedia {
    let mut media = media_bytes("deep");
    for _ in 0..levels {
        media = listed(vec![media]);
    }
    media
}

fn load_depth(entries: &[(&str, &str)]) -> i64 {
    let lookup = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("the environment loads")
        .sections
        .mistral_options
        .max_recurse_depth
}

fn params(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    let mut built = IndexMap::new();
    for (name, value) in entries {
        built.insert((*name).to_owned(), value.clone());
    }
    built
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn media_bytes(value: &str) -> SuppliedMedia {
    SuppliedMedia::Bytes(value.as_bytes().to_vec())
}

fn media_file(value: &str, position: usize) -> SuppliedMedia {
    SuppliedMedia::File(SuppliedFile {
        content: value.as_bytes().to_vec(),
        position,
    })
}

fn listed(items: Vec<SuppliedMedia>) -> SuppliedMedia {
    SuppliedMedia::List(items)
}

fn form_for(
    image: &mut SuppliedMedia,
    mask: MaskInput<'_>,
    optional_params: &IndexMap<String, Json>,
) -> Result<ImageEditForm, EditError> {
    transform_image_edit_request(Some("edit it"), Some(image), mask, optional_params, DEPTH)
}

fn empty_params() -> IndexMap<String, Json> {
    params(&[])
}

#[test]
fn constants_match_the_edit_class() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "mask",
            "seed",
            "output_format",
            "safety_tolerance",
            "prompt_upsampling",
            "aspect_ratio",
            "steps",
            "guidance",
            "grow_mask",
            "top",
            "bottom",
            "left",
            "right"
        ]
    );
    assert!(!MULTIPART_FORM_DATA);
}

#[test]
fn mapping_keeps_allowed_values_in_request_order() {
    let mapped = map_openai_params(
        &params(&[
            ("steps", number(30)),
            ("unrelated", text("dropped")),
            ("seed", number(7)),
            ("mask", text("ignored")),
        ]),
        "black_forest_labs/flux-kontext-pro",
        false,
    );

    assert_eq!(
        mapped.keys().map(String::as_str).collect::<Vec<_>>(),
        ["seed", "steps", "output_format"]
    );
    assert_eq!(mapped.get("seed"), Some(&number(7)));
    assert_eq!(mapped.get("steps"), Some(&number(30)));
    assert_eq!(mapped.get("output_format"), Some(&text("png")));
}

#[test]
fn mapping_keeps_an_explicit_output_format() {
    let mapped = map_openai_params(
        &params(&[("output_format", text("jpeg"))]),
        "any-model",
        true,
    );

    assert_eq!(
        mapped.keys().map(String::as_str).collect::<Vec<_>>(),
        ["output_format"]
    );
    assert_eq!(mapped.get("output_format"), Some(&text("jpeg")));
}

#[test]
fn mapping_supplies_png_when_the_format_is_null() {
    let mapped = map_openai_params(
        &params(&[("output_format", Json::Null), ("seed", number(3))]),
        "any-model",
        true,
    );

    assert_eq!(
        mapped.keys().map(String::as_str).collect::<Vec<_>>(),
        ["seed", "output_format"]
    );
    assert_eq!(mapped.get("output_format"), Some(&text("png")));
}

#[test]
fn mapping_ignores_drop_params_and_the_model() {
    let configured = params(&[("seed", number(1)), ("unknown", Json::Bool(true))]);

    let kept = map_openai_params(&configured, "flux-kontext-max", false);
    let dropped = map_openai_params(&configured, "totally-unrelated", true);

    assert_eq!(kept, dropped);
    assert_eq!(
        kept.keys().map(String::as_str).collect::<Vec<_>>(),
        ["seed", "output_format"]
    );
}

#[rstest]
#[case::plain("flux-kontext-pro", "/v1/flux-kontext-pro")]
#[case::max("flux-kontext-max", "/v1/flux-kontext-max")]
#[case::fill("flux-pro-1.0-fill", "/v1/flux-pro-1.0-fill")]
#[case::expand("flux-pro-1.0-expand", "/v1/flux-pro-1.0-expand")]
#[case::prefixed("black_forest_labs/flux-kontext-max", "/v1/flux-kontext-max")]
#[case::uppercased("BLACK_FOREST_LABS/FLUX-PRO-1.0-FILL", "/v1/flux-pro-1.0-fill")]
fn endpoint_resolves_each_edit_model(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(edit_endpoint(model).unwrap(), expected);
}

#[test]
fn endpoint_rejects_an_unknown_last_component() {
    let error = edit_endpoint("black_forest_labs/FLUX-Unknown").unwrap_err();

    assert!(matches!(error, BflError::UnknownModel { model } if model == "flux-unknown"));
}

#[rstest]
#[case::caller_base(
    Some("https://caller.test//"),
    "https://caller.test/v1/flux-kontext-pro"
)]
#[case::empty_caller_base(Some(""), "https://env.test/v1/flux-kontext-pro")]
#[case::configured_base(None, "https://env.test/v1/flux-kontext-pro")]
fn complete_url_joins_the_trimmed_base(#[case] api_base: Option<&str>, #[case] expected: &str) {
    let url = complete_url(api_base, "flux-kontext-pro", "https://env.test/");

    assert_eq!(url.unwrap(), expected);
}

#[test]
fn complete_url_rejects_an_unknown_model() {
    let error = complete_url(None, "flux-pro-1.1", "https://env.test").unwrap_err();

    assert!(matches!(error, BflError::UnknownModel { .. }));
}

#[test]
fn transform_builds_the_bfl_edit_body() {
    let mut image = media_bytes("image");
    let form = form_for(
        &mut image,
        MaskInput::Absent,
        &params(&[("seed", number(7))]),
    )
    .unwrap();

    assert_eq!(
        form.data.keys().map(String::as_str).collect::<Vec<_>>(),
        ["prompt", "input_image", "seed"]
    );
    assert_eq!(form.data.get("prompt"), Some(&text("edit it")));
    assert_eq!(form.data.get("input_image"), Some(&text("aW1hZ2U=")));
    assert!(form.files.is_empty());
}

#[test]
fn transform_keeps_a_null_prompt() {
    let mut image = media_bytes("image");
    let form = transform_image_edit_request(
        None,
        Some(&mut image),
        MaskInput::Absent,
        &empty_params(),
        DEPTH,
    )
    .unwrap();

    assert_eq!(form.data.get("prompt"), Some(&Json::Null));
}

#[test]
fn transform_selects_only_the_first_image() {
    let mut image = listed(vec![
        listed(vec![media_bytes("first")]),
        media_bytes("second"),
    ]);
    let form = form_for(&mut image, MaskInput::Absent, &empty_params()).unwrap();

    assert_eq!(form.data.get("input_image"), Some(&text("Zmlyc3Q=")));
}

#[test]
fn transform_rejects_an_empty_image_list() {
    let mut image = listed(Vec::new());
    let error = form_for(&mut image, MaskInput::Absent, &empty_params()).unwrap_err();

    assert!(matches!(
        error,
        EditError::UnsupportedMedia { field } if field == "image"
    ));
    assert_eq!(error.status(), None);
}

#[test]
fn transform_reads_a_file_from_zero_and_keeps_its_cursor() {
    let mut image = media_file("0123", 2);
    let form = form_for(&mut image, MaskInput::Absent, &empty_params()).unwrap();

    assert_eq!(form.data.get("input_image"), Some(&text("MDEyMw==")));
    assert_eq!(image, media_file("0123", 2));
}

#[test]
fn transform_requires_an_image() {
    let error = transform_image_edit_request(
        Some("edit it"),
        None,
        MaskInput::Absent,
        &empty_params(),
        DEPTH,
    )
    .unwrap_err();

    assert!(matches!(error, EditError::UnsupportedMedia { field } if field == "image"));
}

#[test]
fn transform_reads_a_mask_only_when_the_key_is_present() {
    let mut image = media_bytes("image");
    let mut mask = media_bytes("mask");
    let masked = form_for(&mut image, MaskInput::Media(&mut mask), &empty_params()).unwrap();
    let plain = form_for(&mut image, MaskInput::Absent, &empty_params()).unwrap();

    assert_eq!(masked.data.get("mask"), Some(&text("bWFzaw==")));
    assert_eq!(
        masked.data.keys().map(String::as_str).collect::<Vec<_>>(),
        ["prompt", "input_image", "mask"]
    );
    assert!(!plain.data.contains_key("mask"));
    assert_eq!(
        plain.data.keys().map(String::as_str).collect::<Vec<_>>(),
        ["prompt", "input_image"]
    );
}

#[test]
fn transform_rejects_a_null_mask() {
    let mut image = media_bytes("image");
    let error = form_for(&mut image, MaskInput::Null, &empty_params()).unwrap_err();

    assert!(matches!(error, EditError::UnsupportedMedia { field } if field == "mask"));
    assert_eq!(error.status(), None);
}

#[test]
fn transform_rejects_a_file_path() {
    let mut image = SuppliedMedia::Text("/tmp/input.png".to_owned());
    let error = form_for(&mut image, MaskInput::Absent, &empty_params()).unwrap_err();

    assert!(matches!(error, EditError::UnsupportedMedia { field } if field == "image"));
    assert_eq!(error.param(), None);
}

#[test]
fn transform_rejects_a_remote_image_with_the_policy_error() {
    let mut image = SuppliedMedia::Text("https://example.test/input.png".to_owned());
    let error = form_for(&mut image, MaskInput::Absent, &empty_params()).unwrap_err();

    assert!(matches!(error, EditError::UnsupportedMediaUrl { field } if field == "image"));
    assert_eq!(error.status(), Some(400));
    assert_eq!(error.error_type(), Some("invalid_request_error"));
    assert_eq!(error.param(), Some("image"));
    assert_eq!(error.code(), Some("unsupported_media_url"));
}

#[test]
fn transform_rejects_a_remote_mask_with_the_policy_error() {
    let mut image = media_bytes("image");
    let mut mask = SuppliedMedia::Text("http://example.test/mask.png".to_owned());
    let error = form_for(&mut image, MaskInput::Media(&mut mask), &empty_params()).unwrap_err();

    assert!(matches!(error, EditError::UnsupportedMediaUrl { field } if field == "mask"));
    assert_eq!(error.param(), Some("mask"));
    assert_eq!(error.code(), Some("unsupported_media_url"));
}

#[test]
fn transform_allows_the_last_list_level_at_the_depth_limit() {
    let mut image = listed(vec![listed(vec![media_bytes("deep")])]);
    let form = transform_image_edit_request(
        Some("edit it"),
        Some(&mut image),
        MaskInput::Absent,
        &empty_params(),
        2,
    )
    .unwrap();

    assert_eq!(form.data.get("input_image"), Some(&text("ZGVlcA==")));
}

#[rstest]
#[case::one_level_short(1)]
#[case::no_room(0)]
fn transform_stops_past_the_configured_depth(#[case] max_depth: i64) {
    let mut image = listed(vec![listed(vec![media_bytes("deep")])]);
    let error = transform_image_edit_request(
        Some("edit it"),
        Some(&mut image),
        MaskInput::Absent,
        &empty_params(),
        max_depth,
    )
    .unwrap_err();

    assert!(matches!(error, EditError::MediaDepth { field } if field == "image"));
    assert_eq!(error.status(), None);
}

// the reader's max_depth is litellm.constants.DEFAULT_MAX_RECURSE_DEPTH, which the real loader reads
#[test]
fn bfl_edit_depth_settings() {
    let configured = load_depth(&[("DEFAULT_MAX_RECURSE_DEPTH", "1")]);
    assert_eq!(configured, 1);
    let mut shallow = nested(2);
    let error = transform_image_edit_request(
        Some("edit it"),
        Some(&mut shallow),
        MaskInput::Absent,
        &empty_params(),
        configured,
    )
    .unwrap_err();
    assert!(matches!(error, EditError::MediaDepth { field } if field == "image"));

    let unset = load_depth(&[]);
    assert_eq!(unset, 100);
    let mut at_limit = nested(100);
    let form = transform_image_edit_request(
        Some("edit it"),
        Some(&mut at_limit),
        MaskInput::Absent,
        &empty_params(),
        unset,
    )
    .expect("the hundredth list level still reads");
    assert_eq!(form.data.get("input_image"), Some(&text("ZGVlcA==")));

    let mut past_limit = nested(101);
    let error = transform_image_edit_request(
        Some("edit it"),
        Some(&mut past_limit),
        MaskInput::Absent,
        &empty_params(),
        unset,
    )
    .unwrap_err();
    assert!(matches!(error, EditError::MediaDepth { field } if field == "image"));
}

#[tokio::test]
async fn async_transform_prepares_local_media_like_the_sync_path() {
    let mut image = listed(vec![media_bytes("image")]);
    let mut mask = media_file("mask", 1);
    let configured = params(&[("seed", number(4))]);
    let prepared = async_transform_image_edit_request(
        Some("edit it"),
        Some(&mut image),
        MaskInput::Media(&mut mask),
        &configured,
        DEPTH,
    )
    .await
    .unwrap();
    let direct = form_for_like_sync(&mut image, &mut mask, &configured);

    assert_eq!(prepared, direct);
    assert_eq!(mask, media_file("mask", 1));
}

fn form_for_like_sync(
    image: &mut SuppliedMedia,
    mask: &mut SuppliedMedia,
    params: &IndexMap<String, Json>,
) -> ImageEditForm {
    transform_image_edit_request(
        Some("edit it"),
        Some(image),
        MaskInput::Media(mask),
        params,
        DEPTH,
    )
    .unwrap()
}

#[tokio::test]
async fn async_transform_rejects_a_remote_image_before_transforming() {
    let mut image = listed(vec![SuppliedMedia::Text(
        "https://example.test/in.png".to_owned(),
    )]);
    let error = async_transform_image_edit_request(
        Some("edit it"),
        Some(&mut image),
        MaskInput::Absent,
        &empty_params(),
        DEPTH,
    )
    .await
    .unwrap_err();

    assert!(matches!(error, EditError::UnsupportedMediaUrl { field } if field == "image"));
    assert_eq!(error.code(), Some("unsupported_media_url"));
}

#[tokio::test]
async fn async_transform_rejects_a_remote_mask_before_transforming() {
    let mut image = media_bytes("image");
    let mut mask = SuppliedMedia::Text("https://example.test/mask.png".to_owned());
    let error = async_transform_image_edit_request(
        Some("edit it"),
        Some(&mut image),
        MaskInput::Media(&mut mask),
        &empty_params(),
        DEPTH,
    )
    .await
    .unwrap_err();

    assert!(matches!(error, EditError::UnsupportedMediaUrl { field } if field == "mask"));
    assert_eq!(error.status(), Some(400));
    assert_eq!(error.param(), Some("mask"));
}

#[test]
fn response_projects_one_sample_url_with_the_injected_time() {
    let body = br#"{"status":"Ready","result":{"sample":"https://delivery.test/a.png"}}"#;
    let response = transform_image_edit_response(200, body, NOW).unwrap();

    assert_eq!(response.created, NOW);
    assert_eq!(response.data.len(), 1);
    let datum = &response.data[0];
    assert_eq!(datum.url.as_deref(), Some("https://delivery.test/a.png"));
    assert!(datum.b64_json.is_none());
    assert!(datum.revised_prompt.is_none());
    assert!(datum.provider_specific_fields.is_none());
}

#[rstest]
#[case::missing_result(r#"{"status":"Ready"}"#)]
#[case::empty_result(r#"{"result":{}}"#)]
#[case::null_sample(r#"{"result":{"sample":null}}"#)]
#[case::empty_sample(r#"{"result":{"sample":""}}"#)]
#[case::list_result(r#"{"result":[{"url":"https://delivery.test/a.png"}]}"#)]
fn response_requires_a_truthy_sample(#[case] body: &str) {
    let error = transform_image_edit_response(200, body.as_bytes(), NOW).unwrap_err();

    assert_eq!(error.status(), Some(500));
    assert!(error.to_string().contains("No image URL in BFL result"));
}

#[test]
fn response_keeps_the_upstream_status_when_json_fails() {
    let error = transform_image_edit_response(502, b"not json", NOW).unwrap_err();

    assert_eq!(error.status(), Some(502));
    assert!(
        error
            .to_string()
            .contains("the edit reply could not be read")
    );
}
