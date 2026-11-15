#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, ErrorKind};
use std::net::TcpListener;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use betterlite_openai::image_edits::request::ImageEditForm;
use betterlite_openai::image_generation::reply::ImageUsage;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use tempfile::NamedTempFile;

use super::error::FalEditError;
use super::flux_lora_depth::{self, DEPTH_ENDPOINT};
use super::image_input::{EditImage, read_image_bytes, to_data_url};
use super::params::{GENERIC_OPENAI_PARAMS, map_openai_params};
use super::reply::transform_image_edit_response;
use super::selector::{Config, for_model};
use super::transformation::{EditRequest, complete_url, transform_image_edit_request};
use crate::api::fal::images::decode::MAX_FAL_REPLY_BYTES;
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::settings::{DEFAULT_BASE_URL, FalImageSettings};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG: &[u8] = b"\xff\xd8\xff";
const CATALOG: &str = r#"{
    "fal_ai/high/1024-x-768/openai/gpt-image-2/edit": {"litellm_provider": "fal_ai"},
    "fal_ai/medium/1024-x-768/openai/gpt-image-2/edit": {"litellm_provider": "fal_ai"}
}"#;

fn headers() -> ImageHeaders {
    BTreeMap::from([("x-request-id".to_owned(), "req-1".to_owned())])
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn size(width: i64, height: i64) -> Json {
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(width)),
        ("height".to_owned(), number(height)),
    ]))
}

fn options(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn empty_catalog() -> ModelCatalog {
    ModelCatalog::parse(b"{}", UnknownFields::Reject).expect("a catalog")
}

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("a catalog")
}

fn request<'value>(
    prompt: Option<&'value str>,
    images: &'value mut [Option<EditImage<'value>>],
    mask: &'value mut [Option<EditImage<'value>>],
    options: &'value ImageParams,
) -> EditRequest<'value> {
    EditRequest {
        prompt,
        images,
        mask,
        options,
    }
}

fn mask_url(form: &ImageEditForm) -> Option<&str> {
    match form.data.get("mask_url") {
        Some(Json::Text(url)) => Some(url.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[test]
fn to_data_url_forwards_strings_without_reading_anything() {
    let remote = "https://example.test/images/in.png?x=1";
    assert_eq!(
        to_data_url(&mut EditImage::Text(remote)).expect("a data url"),
        remote
    );
    let local = "/definitely/missing/fal-edit-input.png";
    assert_eq!(
        to_data_url(&mut EditImage::Text(local)).expect("a data url"),
        local
    );
    assert_eq!(
        to_data_url(&mut EditImage::Text("")).expect("a data url"),
        ""
    );
}

#[test]
fn to_data_url_never_opens_a_connection_for_a_remote_string() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a listener");
    listener
        .set_nonblocking(true)
        .expect("a nonblocking listener");
    let address = listener.local_addr().expect("the listener address");
    let remote = format!("http://{address}/images/in.png");
    assert_eq!(
        to_data_url(&mut EditImage::Text(remote.as_str())).expect("a data url"),
        remote
    );
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        other => panic!("to_data_url reached the network: {other:?}"),
    }
}

#[test]
fn to_data_url_encodes_bytes_with_exact_prefixes() {
    assert_eq!(
        to_data_url(&mut EditImage::Bytes(PNG)).expect("a data url"),
        "data:image/png;base64,iVBORw0KGgo="
    );
    assert_eq!(
        to_data_url(&mut EditImage::Bytes(JPEG)).expect("a data url"),
        "data:image/jpeg;base64,/9j/"
    );
}

#[rstest]
#[case::gif(b"GIF89a", "image/gif")]
#[case::webp(b"RIFF\x00\x00\x00\x00WEBP", "image/webp")]
#[case::heic(b"\x00\x00\x00\x18ftypheic", "image/heic")]
#[case::unknown(b"not an image", "image/png")]
#[case::empty(b"", "image/png")]
fn to_data_url_picks_the_mime_from_the_leading_bytes(#[case] bytes: &[u8], #[case] mime: &str) {
    let encoded = to_data_url(&mut EditImage::Bytes(bytes)).expect("a data url");
    assert!(
        encoded.starts_with(&format!("data:{mime};base64,")),
        "{encoded}"
    );
    assert!(encoded.ends_with(&BASE64.encode(bytes)));
}

#[test]
fn read_image_bytes_reads_a_local_path() {
    let file = NamedTempFile::new().expect("a temp file");
    fs::write(file.path(), JPEG).expect("the fixture");
    let mut image = EditImage::Path(file.path());
    let bytes = read_image_bytes(&mut image).expect("the file bytes");
    assert_eq!(bytes.as_ref(), JPEG);
}

#[test]
fn read_image_bytes_reports_an_unreadable_path() {
    let path = Path::new("/definitely/missing/fal-edit-input.png");
    let mut image = EditImage::Path(path);
    match read_image_bytes(&mut image) {
        Err(FalEditError::ReadPath { path: carried, .. }) => assert_eq!(carried.as_path(), path),
        other => panic!("{other:?}"),
    }
}

#[test]
fn read_image_bytes_restores_the_reader_cursor() {
    let mut cursor = Cursor::new(JPEG.to_vec());
    cursor.set_position(2);
    let bytes = {
        let mut image = EditImage::Reader(&mut cursor);
        read_image_bytes(&mut image)
            .expect("the reader bytes")
            .into_owned()
    };
    assert_eq!(bytes.as_slice(), JPEG);
    assert_eq!(cursor.position(), 2);
}

#[test]
fn tuple_conversion_reads_the_second_element_recursively() {
    let mut inner = EditImage::Bytes(JPEG);
    let mut tuple = EditImage::Tuple(&mut inner);
    assert_eq!(
        to_data_url(&mut tuple).expect("a data url"),
        "data:image/jpeg;base64,/9j/"
    );
    let mut outer = EditImage::Tuple(&mut tuple);
    assert_eq!(read_image_bytes(&mut outer).expect("bytes").as_ref(), JPEG);
    let mut text_image = EditImage::Text("https://example.test/in.png");
    let mut text_tuple = EditImage::Tuple(&mut text_image);
    match read_image_bytes(&mut text_tuple) {
        Err(FalEditError::UnsupportedImage { kind }) => assert_eq!(kind, "str"),
        other => panic!("{other:?}"),
    }
}

#[rstest]
#[case::list(Json::List(Vec::new()), "list")]
#[case::object(Json::Object(IndexMap::new()), "dict")]
#[case::null(Json::Null, "NoneType")]
#[case::boolean(Json::Bool(true), "bool")]
#[case::integer(number(7), "int")]
#[case::float(Json::Number(Number::from_f64(1.5).expect("a number")), "float")]
fn read_image_bytes_refuses_other_shapes(#[case] value: Json, #[case] kind: &str) {
    let mut image = EditImage::Other(&value);
    match read_image_bytes(&mut image) {
        Err(FalEditError::UnsupportedImage { kind: carried }) => assert_eq!(carried, kind),
        other => panic!("{other:?}"),
    }
}

#[test]
fn generic_supported_params_are_advertised() {
    assert_eq!(
        GENERIC_OPENAI_PARAMS,
        ["background", "mask", "n", "quality", "size"]
    );
}

#[test]
fn generic_mapping_translates_the_known_options() {
    let mapped = map_openai_params(
        &options(&[
            ("background", text("transparent")),
            ("n", number(2)),
            ("size", text("1024x768")),
        ]),
        "openai/gpt-image-2/edit",
        &empty_catalog(),
    );
    assert_eq!(
        mapped,
        options(&[
            ("background", text("transparent")),
            ("num_images", number(2)),
            ("image_size", size(1024, 768)),
        ])
    );
}

#[test]
fn generic_mapping_keeps_unknown_options_and_skips_nulls() {
    let mapped = map_openai_params(
        &options(&[
            ("user", text("u-1")),
            ("response_format", text("b64_json")),
            ("seed", number(7)),
            ("n", Json::Null),
        ]),
        "fal-ai/any",
        &empty_catalog(),
    );
    assert_eq!(
        mapped,
        options(&[
            ("user", text("u-1")),
            ("response_format", text("b64_json")),
            ("seed", number(7)),
        ])
    );
}

#[test]
fn generic_mapping_lets_a_later_key_replace_the_same_destination() {
    let mapped = map_openai_params(
        &options(&[("n", number(2)), ("num_images", number(5))]),
        "fal-ai/any",
        &empty_catalog(),
    );
    assert_eq!(mapped, options(&[("num_images", number(5))]));
}

#[rstest]
#[case::alias_hd("hd", "high")]
#[case::alias_standard("standard", "medium")]
#[case::plain_high("high", "high")]
#[case::unsupported_low("low", "auto")]
#[case::auto("auto", "auto")]
fn generic_mapping_applies_the_catalog_quality_policy(
    #[case] quality: &str,
    #[case] expected: &str,
) {
    let mapped = map_openai_params(
        &options(&[("quality", text(quality))]),
        "openai/gpt-image-2/edit",
        &catalog(),
    );
    assert_eq!(mapped.get("quality"), Some(&text(expected)));
}

#[test]
fn generic_mapping_keeps_quality_without_a_catalog_entry() {
    let mapped = map_openai_params(
        &options(&[("quality", text("standard"))]),
        "fal-ai/unknown/edit",
        &empty_catalog(),
    );
    assert_eq!(mapped.get("quality"), Some(&text("medium")));
}

#[rstest]
#[case::plain(text("512x768"), size(512, 768))]
#[case::auto(text("auto"), text("auto"))]
#[case::broken(text("big"), text("big"))]
#[case::non_string(number(3), number(3))]
fn generic_mapping_maps_size_through_the_gpt_parser(#[case] input: Json, #[case] expected: Json) {
    let mapped = map_openai_params(&options(&[("size", input)]), "fal-ai/any", &empty_catalog());
    assert_eq!(mapped.get("image_size"), Some(&expected));
}

#[rstest]
#[case::plain("openai/gpt-image-2.5/flare", "openai/gpt-image-2.5/flare/edit")]
#[case::already("openai/gpt-image-2/edit", "openai/gpt-image-2/edit")]
#[case::bare("fal-ai/flux", "fal-ai/flux/edit")]
#[case::word("edit", "edit/edit")]
#[case::upper_suffix("fal-ai/x/EDIT", "fal-ai/x/EDIT/edit")]
fn generic_url_appends_the_edit_suffix_only_when_absent(#[case] model: &str, #[case] path: &str) {
    assert_eq!(
        complete_url(&FalImageSettings::default(), None, model),
        format!("{DEFAULT_BASE_URL}/{path}")
    );
}

#[test]
fn generic_url_prefers_an_explicit_base_and_trims_it() {
    let environment = FalImageSettings {
        api_key: None,
        api_base: "https://environment.test/fal/".to_owned(),
    };
    assert_eq!(
        complete_url(&environment, Some("https://explicit.test/fal///"), "m"),
        "https://explicit.test/fal/m/edit"
    );
    assert_eq!(
        complete_url(&environment, Some(""), "m"),
        "https://environment.test/fal/m/edit"
    );
    assert_eq!(
        complete_url(&environment, None, "m"),
        "https://environment.test/fal/m/edit"
    );
}

#[rstest]
#[case::exact("fal-ai/flux-lora-depth", Config::FluxLoraDepth)]
#[case::mixed("FAL-AI/FLUX-LORA-DEPTH", Config::FluxLoraDepth)]
#[case::edit_suffix("fal-ai/flux-lora-depth/edit", Config::FluxLoraDepth)]
#[case::embedded("vendor/flux-lora-depth-v2", Config::FluxLoraDepth)]
#[case::generic("fal-ai/flux/schnell", Config::Generic)]
#[case::partial("fal-ai/flux-lora", Config::Generic)]
fn selector_picks_the_depth_config_for_the_lora_marker(
    #[case] model: &str,
    #[case] expected: Config,
) {
    assert_eq!(for_model(model), expected);
}

#[test]
fn depth_supported_params_are_advertised() {
    assert_eq!(flux_lora_depth::OPENAI_PARAMS, ["n", "size"]);
}

#[rstest]
#[case::exact("fal-ai/flux-lora-depth")]
#[case::edit_suffix("fal-ai/flux-lora-depth/edit")]
#[case::mixed_case("fal-ai/Flux-Lora-Depth/edit")]
#[case::embedded("vendor/flux-lora-depth-v2")]
fn depth_url_ignores_the_model_and_appends_the_depth_endpoint(#[case] model: &str) {
    let settings = FalImageSettings::default();
    assert_eq!(
        for_model(model).complete_url(&settings, None, model),
        "https://fal.run/fal-ai/flux-lora-depth"
    );
}

#[test]
fn depth_url_keeps_an_explicit_base_ending_in_a_slash() {
    let model = "fal-ai/Flux-Lora-Depth/edit";
    assert_eq!(
        for_model(model).complete_url(
            &FalImageSettings::default(),
            Some("https://explicit.test/"),
            model
        ),
        "https://explicit.test/fal-ai/flux-lora-depth"
    );
}

#[test]
fn generic_url_through_the_selector_appends_the_edit_suffix() {
    let model = "fal-ai/flux/schnell";
    assert_eq!(
        for_model(model).complete_url(&FalImageSettings::default(), None, model),
        "https://fal.run/fal-ai/flux/schnell/edit"
    );
}

#[test]
fn depth_mapping_keeps_only_n_and_size() {
    let mapped = flux_lora_depth::map_openai_params(&options(&[
        ("n", number(1)),
        ("size", text("1024x1024")),
        ("quality", text("high")),
        ("background", text("auto")),
        ("seed", number(3)),
    ]));
    assert_eq!(
        mapped,
        options(&[("num_images", number(1)), ("image_size", size(1024, 1024)),])
    );
}

#[test]
fn depth_mapping_skips_null_values() {
    let mapped =
        flux_lora_depth::map_openai_params(&options(&[("n", Json::Null), ("size", Json::Null)]));
    assert!(mapped.is_empty());
}

#[test]
fn generic_body_lists_every_non_null_image() {
    let remote = "https://example.test/in.png";
    let local = EditImage::Bytes(PNG);
    let linked = EditImage::Text(remote);
    let mut images = [None, Some(local), Some(linked)];
    let options = options(&[]);
    let form =
        transform_image_edit_request(&mut request(Some("paint"), &mut images, &mut [], &options))
            .expect("a form");
    assert_eq!(form.data.get("prompt"), Some(&text("paint")));
    assert_eq!(
        form.data.get("image_urls"),
        Some(&Json::List(vec![
            text("data:image/png;base64,iVBORw0KGgo="),
            text(remote),
        ]))
    );
    assert!(form.files.is_empty());
}

#[test]
fn generic_body_uses_a_null_prompt_when_none_is_given() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let options = options(&[]);
    let form = transform_image_edit_request(&mut request(None, &mut images, &mut [], &options))
        .expect("a form");
    assert_eq!(form.data.get("prompt"), Some(&Json::Null));
}

#[test]
fn generic_edit_requires_at_least_one_image() {
    let mut none_images: [Option<EditImage<'_>>; 0] = [];
    let options = ImageParams::new();
    assert!(matches!(
        transform_image_edit_request(&mut request(None, &mut none_images, &mut [], &options)),
        Err(FalEditError::MissingImages)
    ));
    let mut skipped = [None];
    assert!(matches!(
        transform_image_edit_request(&mut request(None, &mut skipped, &mut [], &options)),
        Err(FalEditError::MissingImages)
    ));
}

#[rstest]
#[case::absent(Vec::new(), None)]
#[case::null(vec![("mask", Json::Null)], None)]
#[case::string(
    vec![("mask", text("https://mask.test/m.png"))],
    Some("https://mask.test/m.png")
)]
#[case::list_first(
    vec![(
        "mask",
        Json::List(vec![
            text("https://mask.test/first.png"),
            text("https://mask.test/second.png"),
        ])
    )],
    Some("https://mask.test/first.png")
)]
#[case::list_first_null(
    vec![(
        "mask",
        Json::List(vec![Json::Null, text("https://mask.test/second.png")])
    )],
    None
)]
fn generic_mask_takes_the_first_list_element_or_the_value(
    #[case] entries: Vec<(&str, Json)>,
    #[case] expected: Option<&str>,
) {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let options = options(&entries);
    let form = transform_image_edit_request(&mut request(None, &mut images, &mut [], &options))
        .expect("a form");
    assert_eq!(mask_url(&form), expected);
}

#[test]
fn generic_mask_refuses_an_empty_list() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let options = options(&[("mask", Json::List(Vec::new()))]);
    match transform_image_edit_request(&mut request(None, &mut images, &mut [], &options)) {
        Err(FalEditError::UnsupportedImage { kind }) => assert_eq!(kind, "list"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn generic_mask_refuses_a_nested_list() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let options = options(&[("mask", Json::List(vec![Json::List(vec![text("x")])]))]);
    match transform_image_edit_request(&mut request(None, &mut images, &mut [], &options)) {
        Err(FalEditError::UnsupportedImage { kind }) => assert_eq!(kind, "list"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn uploaded_mask_entries_become_a_data_url() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let mask_image = EditImage::Bytes(JPEG);
    let mut mask = [Some(mask_image)];
    let options = options(&[]);
    let form = transform_image_edit_request(&mut request(None, &mut images, &mut mask, &options))
        .expect("a form");
    assert_eq!(mask_url(&form), Some("data:image/jpeg;base64,/9j/"));
}

#[test]
fn uploaded_mask_readers_become_a_data_url_and_keep_their_cursor() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let mut cursor = Cursor::new(JPEG.to_vec());
    cursor.set_position(2);
    let mut mask = [Some(EditImage::Reader(&mut cursor))];
    let options = options(&[]);
    let form = transform_image_edit_request(&mut request(None, &mut images, &mut mask, &options))
        .expect("a form");
    assert_eq!(mask_url(&form), Some("data:image/jpeg;base64,/9j/"));
    assert_eq!(cursor.position(), 2);
}

#[test]
fn uploaded_mask_entries_win_over_the_json_mask() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let mask_image = EditImage::Bytes(JPEG);
    let mut mask = [Some(mask_image)];
    let options = options(&[("mask", text("https://mask.test/from-option.png"))]);
    let form = transform_image_edit_request(&mut request(None, &mut images, &mut mask, &options))
        .expect("a form");
    assert_eq!(mask_url(&form), Some("data:image/jpeg;base64,/9j/"));
}

#[test]
fn a_null_first_uploaded_mask_entry_skips_the_json_mask() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let mut mask = [None];
    let options = options(&[("mask", text("https://mask.test/from-option.png"))]);
    let form = transform_image_edit_request(&mut request(None, &mut images, &mut mask, &options))
        .expect("a form");
    assert_eq!(mask_url(&form), None);
}

#[test]
fn a_mask_conversion_error_wins_over_an_image_error() {
    let unsupported = Json::Object(IndexMap::new());
    let mut images = [Some(EditImage::Other(&unsupported))];
    let options = options(&[("mask", Json::List(Vec::new()))]);
    match transform_image_edit_request(&mut request(None, &mut images, &mut [], &options)) {
        Err(FalEditError::UnsupportedImage { kind }) => assert_eq!(kind, "list"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn provider_options_overlay_the_generated_fields() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let overrides = options(&[
        ("prompt", text("replacement")),
        (
            "image_urls",
            Json::List(vec![text("https://override.test/one.png")]),
        ),
        ("mask_url", text("https://override.test/mask.png")),
        ("mask", text("https://mask.test/from-option.png")),
        ("seed", number(4)),
    ]);
    let form = transform_image_edit_request(&mut request(
        Some("original"),
        &mut images,
        &mut [],
        &overrides,
    ))
    .expect("a form");
    assert_eq!(form.data.get("prompt"), Some(&text("replacement")));
    assert_eq!(
        form.data.get("image_urls"),
        Some(&Json::List(vec![text("https://override.test/one.png")]))
    );
    assert_eq!(mask_url(&form), Some("https://override.test/mask.png"));
    assert_eq!(form.data.get("seed"), Some(&number(4)));
    assert!(!form.data.contains_key("mask"));
}

#[test]
fn depth_body_uses_a_singular_image_url_and_drops_the_mask() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let depth_options = options(&[
        ("num_images", number(1)),
        ("mask", text("https://mask.test/m.png")),
        ("seed", number(9)),
    ]);
    let form = flux_lora_depth::transform_image_edit_request(&mut request(
        Some("paint"),
        &mut images,
        &mut [],
        &depth_options,
    ))
    .expect("a form");
    let expected = options(&[
        ("prompt", text("paint")),
        ("image_url", text("data:image/png;base64,iVBORw0KGgo=")),
        ("num_images", number(1)),
        ("seed", number(9)),
    ]);
    assert_eq!(form.data, expected);
    assert!(form.files.is_empty());
}

#[test]
fn depth_provider_options_overlay_the_body() {
    let image = EditImage::Bytes(PNG);
    let mut images = [Some(image)];
    let depth_options = options(&[
        ("prompt", text("replacement")),
        ("image_url", text("https://override.test/control.png")),
    ]);
    let form = flux_lora_depth::transform_image_edit_request(&mut request(
        Some("original"),
        &mut images,
        &mut [],
        &depth_options,
    ))
    .expect("a form");
    assert_eq!(form.data.get("prompt"), Some(&text("replacement")));
    assert_eq!(
        form.data.get("image_url"),
        Some(&text("https://override.test/control.png"))
    );
    assert!(!form.data.contains_key("image_urls"));
}

#[test]
fn depth_requires_a_control_image() {
    let mut none_images: [Option<EditImage<'_>>; 0] = [];
    let options = ImageParams::new();
    assert!(matches!(
        flux_lora_depth::transform_image_edit_request(&mut request(
            None,
            &mut none_images,
            &mut [],
            &options
        )),
        Err(FalEditError::MissingImages)
    ));
    let mut skipped = [None];
    assert!(matches!(
        flux_lora_depth::transform_image_edit_request(&mut request(
            None,
            &mut skipped,
            &mut [],
            &options
        )),
        Err(FalEditError::MissingImages)
    ));
}

#[test]
fn depth_rejects_more_than_one_control_image() {
    let first = EditImage::Bytes(PNG);
    let second = EditImage::Bytes(JPEG);
    let mut images = [Some(first), Some(second)];
    match flux_lora_depth::transform_image_edit_request(&mut request(
        None,
        &mut images,
        &mut [],
        &ImageParams::new(),
    )) {
        Err(FalEditError::ControlImages { endpoint }) => assert_eq!(endpoint, DEPTH_ENDPOINT),
        other => panic!("{other:?}"),
    }
}

#[test]
fn depth_counts_only_non_null_images() {
    let kept = EditImage::Bytes(PNG);
    let mut one = [None, Some(kept)];
    assert!(
        flux_lora_depth::transform_image_edit_request(&mut request(
            None,
            &mut one,
            &mut [],
            &ImageParams::new()
        ))
        .is_ok()
    );
    let first = EditImage::Bytes(PNG);
    let second = EditImage::Bytes(JPEG);
    let mut two = [Some(first), None, Some(second)];
    assert!(matches!(
        flux_lora_depth::transform_image_edit_request(&mut request(
            None,
            &mut two,
            &mut [],
            &ImageParams::new()
        )),
        Err(FalEditError::ControlImages { .. })
    ));
}

#[test]
fn response_projects_images_and_the_injected_created_time() {
    let body = br#"{"images":[{"url":"https://fal.test/1.png","width":512,"height":768,"content_type":"image/png"},"https://fal.test/2.png",true,{"b64_json":"iVBORw0KGgo="}]}"#;
    let response =
        transform_image_edit_response(200, &headers(), body, 1_700_000_000).expect("a response");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.usage, ImageUsage::default());
    assert_eq!(response.background, None);
    assert_eq!(response.quality, None);
    assert_eq!(response.output_format, None);
    assert_eq!(response.size, None);
    assert_eq!(response.data.len(), 3);
    assert_eq!(
        response.data[0].url.as_deref(),
        Some("https://fal.test/1.png")
    );
    assert_eq!(response.data[0].b64_json, None);
    assert_eq!(response.data[0].revised_prompt, None);
    let provider = response.data[0]
        .provider_specific_fields
        .as_ref()
        .expect("provider fields");
    assert_eq!(provider.get("width"), Some(&number(512)));
    assert_eq!(provider.get("height"), Some(&number(768)));
    assert_eq!(provider.get("content_type"), Some(&text("image/png")));
    assert_eq!(
        response.data[1].url.as_deref(),
        Some("https://fal.test/2.png")
    );
    assert_eq!(response.data[1].provider_specific_fields, None);
    assert_eq!(response.data[2].b64_json.as_deref(), Some("iVBORw0KGgo="));
    assert_eq!(response.data[2].url, None);
}

#[test]
fn response_without_images_starts_an_empty_list() {
    let empty =
        transform_image_edit_response(200, &headers(), br#"{"images":[]}"#, 5).expect("a response");
    assert_eq!(empty.created, 5);
    assert!(empty.data.is_empty());
    let absent = transform_image_edit_response(200, &headers(), br#"{}"#, 5).expect("a response");
    assert!(absent.data.is_empty());
    let not_a_list = transform_image_edit_response(
        200,
        &headers(),
        br#"{"images":{"url":"https://fal.test/1.png"}}"#,
        5,
    )
    .expect("a response");
    assert!(not_a_list.data.is_empty());
}

#[test]
fn non_json_reply_names_the_edit_response_and_keeps_the_status() {
    match transform_image_edit_response(502, &headers(), b"<html>", 0) {
        Err(FalEditError::Reply {
            source:
                FalImageError::ReplyDecode {
                    status,
                    headers: carried,
                    ..
                },
        }) => {
            assert_eq!(status, 502);
            assert_eq!(carried, headers());
        }
        other => panic!("{other:?}"),
    }
    let error =
        transform_image_edit_response(200, &headers(), b"<html>", 0).expect_err("a reply error");
    assert!(
        error
            .to_string()
            .starts_with("Error parsing Fal AI image edit response:")
    );
}

#[test]
fn non_object_reply_stays_a_typed_error() {
    match transform_image_edit_response(200, &headers(), b"[]", 0) {
        Err(FalEditError::Reply {
            source: FalImageError::ReplyNotAnObject { status, .. },
        }) => assert_eq!(status, 200),
        other => panic!("{other:?}"),
    }
}

#[test]
fn oversized_reply_stays_a_typed_error() {
    let oversized = vec![b' '; MAX_FAL_REPLY_BYTES + 1];
    match transform_image_edit_response(200, &headers(), &oversized, 0) {
        Err(FalEditError::Reply {
            source: FalImageError::ReplyTooLarge { status, limit, .. },
        }) => {
            assert_eq!(status, 200);
            assert_eq!(limit, MAX_FAL_REPLY_BYTES);
        }
        other => panic!("{other:?}"),
    }
}
