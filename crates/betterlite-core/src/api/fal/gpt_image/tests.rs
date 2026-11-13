#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use indexmap::IndexMap;
use rstest::rstest;

use super::GPT_OPENAI_PARAMS;
use super::complete_url;
use super::params::map_openai_params;
use super::quality::{map_gpt_image_quality, supported_gpt_image_qualities};
use super::size::map_gpt_image_size;
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::settings::{DEFAULT_BASE_URL, FalImageSettings};

const CATALOG: &str = r#"{
    "fal_ai/high/1024-x-768/openai/gpt-image-2": {"litellm_provider": "fal_ai"},
    "fal_ai/low/1024-x-768/openai/gpt-image-2": {"litellm_provider": "fal_ai"},
    "fal_ai/medium/1024-x-768/gpt-image-2": {"litellm_provider": "fal_ai"},
    "fal_ai/high/1024-x-768/openai/gpt-image-2/edit": {"litellm_provider": "fal_ai"},
    "fal_ai/high/1024-x-768/openai/elsewhere": {"litellm_provider": "fal_ai"},
    "fal_ai/short": {"litellm_provider": "fal_ai"},
    "fal_ai/ultra/1024x768/openai/gpt-image-2": {"litellm_provider": "fal_ai"},
    "other/medium/1024-x-768/openai/gpt-image-2": {"litellm_provider": "other"}
}"#;

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("a catalog")
}

fn settings() -> FalImageSettings {
    FalImageSettings::default()
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn size(width: i64, height: i64) -> Json {
    Json::Object(IndexMap::from([
        ("width".to_owned(), number(width)),
        ("height".to_owned(), number(height)),
    ]))
}

#[test]
fn gpt_url_prepends_the_openai_prefix_once() {
    assert_eq!(
        complete_url(&settings(), None, "gpt-image-2"),
        format!("{DEFAULT_BASE_URL}/openai/gpt-image-2")
    );
    assert_eq!(
        complete_url(&settings(), None, "openai/gpt-image-2"),
        format!("{DEFAULT_BASE_URL}/openai/gpt-image-2")
    );
    assert_eq!(
        complete_url(&settings(), None, "openai/gpt-image-2/edit"),
        format!("{DEFAULT_BASE_URL}/openai/gpt-image-2/edit")
    );
}

#[test]
fn gpt_url_trims_the_base_and_prefers_an_explicit_one() {
    assert_eq!(
        complete_url(
            &settings(),
            Some("https://proxy.test/fal///"),
            "gpt-image-2"
        ),
        "https://proxy.test/fal/openai/gpt-image-2"
    );
    let environment = FalImageSettings {
        api_key: None,
        api_base: "https://environment.test/fal/".to_owned(),
    };
    assert_eq!(
        complete_url(&environment, None, "gpt-image-2"),
        "https://environment.test/fal/openai/gpt-image-2"
    );
}

#[rstest]
#[case::plain("1024x768", size(1024, 768))]
#[case::upper("1024X768", size(1024, 768))]
#[case::spaced(" 10 x 20 ", size(10, 20))]
#[case::signed("-5x+6", size(-5, 6))]
#[case::underscored("1_0x20", size(10, 20))]
#[case::auto("auto", text("auto"))]
#[case::auto_upper("AUTO", text("AUTO"))]
#[case::three_parts("10x20x30", text("10x20x30"))]
#[case::missing_height("10x", text("10x"))]
#[case::fraction("10.5x20", text("10.5x20"))]
#[case::words("abcxdef", text("abcxdef"))]
#[case::empty("", text(""))]
#[case::two_spellings("1__0x20", text("1__0x20"))]
fn gpt_size_maps_only_two_plain_integer_dimensions(#[case] value: &str, #[case] expected: Json) {
    assert_eq!(map_gpt_image_size(&text(value)), expected);
}

#[rstest]
#[case::number(number(7))]
#[case::object(size(5, 6))]
#[case::null(Json::Null)]
#[case::truth(Json::Bool(true))]
fn gpt_size_passes_a_nonstring_through(#[case] value: Json) {
    assert_eq!(map_gpt_image_size(&value), value);
}

#[test]
fn gpt_qualities_come_from_the_matching_endpoint_rows() {
    assert_eq!(
        supported_gpt_image_qualities("gpt-image-2", &catalog()),
        BTreeSet::from(["auto".to_owned(), "high".to_owned(), "low".to_owned()])
    );
    assert_eq!(
        supported_gpt_image_qualities("fal_ai/openai/gpt-image-2", &catalog()),
        BTreeSet::from(["auto".to_owned(), "high".to_owned(), "low".to_owned()])
    );
    assert_eq!(
        supported_gpt_image_qualities("openai/gpt-image-2/edit", &catalog()),
        BTreeSet::from(["auto".to_owned(), "high".to_owned()])
    );
    assert!(supported_gpt_image_qualities("gpt-image-2.5", &catalog()).is_empty());
}

#[test]
fn gpt_quality_keeps_the_normalized_quality_without_rows() {
    let empty = ModelCatalog::parse(b"{}", UnknownFields::Reject).expect("a catalog");
    assert_eq!(
        map_gpt_image_quality(&text("hd"), "gpt-image-2", &empty),
        text("high")
    );
    assert_eq!(
        map_gpt_image_quality(&text("standard"), "gpt-image-2", &empty),
        text("medium")
    );
    assert_eq!(
        map_gpt_image_quality(&text("custom"), "gpt-image-2", &empty),
        text("custom")
    );
}

#[test]
fn gpt_quality_replaces_an_unsupported_value_with_auto() {
    let catalog = catalog();
    assert_eq!(
        map_gpt_image_quality(&text("hd"), "gpt-image-2", &catalog),
        text("high")
    );
    assert_eq!(
        map_gpt_image_quality(&text("low"), "gpt-image-2", &catalog),
        text("low")
    );
    assert_eq!(
        map_gpt_image_quality(&text("auto"), "gpt-image-2", &catalog),
        text("auto")
    );
    assert_eq!(
        map_gpt_image_quality(&text("standard"), "gpt-image-2", &catalog),
        text("auto")
    );
    assert_eq!(
        map_gpt_image_quality(&text("HIGH"), "gpt-image-2", &catalog),
        text("auto")
    );
    assert_eq!(
        map_gpt_image_quality(&number(5), "gpt-image-2", &catalog),
        number(5)
    );
}

#[test]
fn gpt_supported_params_are_the_five_openai_names() {
    assert_eq!(
        GPT_OPENAI_PARAMS,
        ["n", "output_format", "quality", "response_format", "size"]
    );
}

#[test]
fn gpt_mapping_translates_n_size_and_output_format() {
    let non_default = params(&[
        ("n", number(2)),
        ("size", text("1024x768")),
        ("output_format", text("png")),
    ]);
    let mapped = map_openai_params(
        &non_default,
        ImageParams::new(),
        "gpt-image-2",
        false,
        &catalog(),
    )
    .expect("supported params");
    assert_eq!(
        mapped,
        params(&[
            ("num_images", number(2)),
            ("image_size", size(1024, 768)),
            ("output_format", text("png")),
        ])
    );
}

#[test]
fn gpt_mapping_checks_the_translated_destination() {
    let non_default = params(&[("n", number(2)), ("size", text("1024x768"))]);
    let optional = params(&[("num_images", number(3)), ("size", text("auto"))]);
    let mapped = map_openai_params(&non_default, optional, "gpt-image-2", false, &catalog())
        .expect("a destination");
    assert_eq!(
        mapped,
        params(&[
            ("num_images", number(3)),
            ("size", text("auto")),
            ("image_size", size(1024, 768)),
        ])
    );
}

#[test]
fn gpt_mapping_advertises_response_format_without_translating_it() {
    let non_default = params(&[("response_format", text("url"))]);
    let mapped = map_openai_params(
        &non_default,
        ImageParams::new(),
        "gpt-image-2",
        false,
        &catalog(),
    )
    .expect("an advertised param");
    assert!(mapped.is_empty());
}

#[test]
fn gpt_mapping_rejects_unsupported_params_without_drop_params() {
    let non_default = params(&[("seed", number(1)), ("user", text("alice"))]);
    match map_openai_params(
        &non_default,
        ImageParams::new(),
        "gpt-image-2",
        false,
        &catalog(),
    ) {
        Err(FalImageError::UnsupportedParameters { parameters, model }) => {
            assert_eq!(parameters, ["seed", "user"]);
            assert_eq!(model, "gpt-image-2");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn gpt_mapping_drops_unsupported_params_with_drop_params() {
    let non_default = params(&[("seed", number(1)), ("n", number(2))]);
    let mapped = map_openai_params(
        &non_default,
        ImageParams::new(),
        "gpt-image-2",
        true,
        &catalog(),
    )
    .expect("dropped params");
    assert_eq!(mapped, params(&[("num_images", number(2))]));
}

#[test]
fn gpt_mapping_keeps_an_unsupported_param_that_is_already_present() {
    let non_default = params(&[("seed", number(1))]);
    let optional = params(&[("seed", number(9))]);
    let mapped = map_openai_params(&non_default, optional, "gpt-image-2", false, &catalog())
        .expect("an existing param");
    assert_eq!(mapped.get("seed"), Some(&number(9)));
}

#[test]
fn gpt_mapping_translates_quality_through_the_catalog() {
    let non_default = params(&[("quality", text("hd"))]);
    let mapped = map_openai_params(
        &non_default,
        ImageParams::new(),
        "gpt-image-2",
        false,
        &catalog(),
    )
    .expect("a translated quality");
    assert_eq!(mapped.get("quality"), Some(&text("high")));
    let with_existing = map_openai_params(
        &non_default,
        params(&[("quality", text("low"))]),
        "gpt-image-2",
        false,
        &catalog(),
    )
    .expect("an existing quality");
    assert_eq!(with_existing.get("quality"), Some(&text("low")));
}
