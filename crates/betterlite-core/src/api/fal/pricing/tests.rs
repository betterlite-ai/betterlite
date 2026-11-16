#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::keyed::{keyed_quality, keyed_size, parse_keyed_dimensions};
use super::{
    DEFAULT_KEYED_DIMENSIONS, FAL_AI_QUEUE_DEFAULT_BASE, FAL_KEYED_DEFAULT_QUALITY,
    FAL_NAMED_IMAGE_SIZES, FAL_PIXELS_PER_MEGAPIXEL, FAL_TEXT_TO_IMAGE_DEFAULT_SIZE,
    FalCostRequest, FalImageResponse, FalPricingError, cost_calculator, fal_ai_queue_base,
    image_dimensions, resolution_key,
};

mod passthrough;

const SKIP_FLAT: &str = r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.75}"#;

const FLUX_SCHNELL: &str = "fal_ai/fal-ai/flux/schnell";
const FLUX_DEV: &str = "fal_ai/fal-ai/flux/dev";

const FLAT_SCHNELL: &str = r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.5}"#;
const FLAT_DEV: &str = r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.5, "output_cost_per_image_1024": 0.125}"#;

struct Lookup {
    entries: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for Lookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let scoped =
            custom_llm_provider.and_then(|name| self.entries.get(&format!("{name}/{model}")));
        scoped.or_else(|| self.entries.get(model)).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        priced(self.effective_model_info(model, custom_llm_provider))
    }
}

fn priced(found: Option<PriceRecord>) -> Option<PricedModel> {
    let record = found?;
    Some(PricedModel {
        record,
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    })
}

struct Fixture {
    lookup: Lookup,
    catalog: ModelCatalog,
}

fn lookup_from(entries: &[(&str, &str)]) -> Lookup {
    Lookup {
        entries: entries
            .iter()
            .map(|(key, body)| {
                (
                    (*key).to_owned(),
                    serde_json::from_str::<PriceRecord>(body).expect("the entry is a price record"),
                )
            })
            .collect(),
    }
}

fn fixture(entries: &[(&str, &str)]) -> Fixture {
    let mut catalog_entries: BTreeMap<String, Json> = BTreeMap::new();
    for (key, body) in entries {
        let value: Json = serde_json::from_str(body).expect("the entry is a JSON object");
        catalog_entries.insert((*key).to_owned(), value);
    }
    let bytes = serde_json::to_vec(&catalog_entries).expect("the catalog serializes");
    Fixture {
        lookup: lookup_from(entries),
        catalog: ModelCatalog::parse(&bytes, UnknownFields::Reject).expect("the catalog parses"),
    }
}

fn decimal(text: &str) -> Decimal {
    text.parse().expect("a decimal literal")
}

fn record(entries: &[(&str, &str)]) -> PriceRecord {
    let parse = |body: &str| serde_json::from_str::<Scalar>(body).expect("a price scalar");
    entries
        .iter()
        .map(|(key, body)| ((*key).to_owned(), parse(body)))
        .collect()
}

fn integer(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn number(text: &str) -> Json {
    Json::Number(text.parse().expect("a numeric literal"))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn priced_fixture(model: &str, flat: &str, rows: &[(&str, &str)]) -> Fixture {
    let mut entries = vec![(format!("fal_ai/{model}"), flat.to_owned())];
    for (size, price) in rows {
        let body = format!(r#"{{"litellm_provider": "fal_ai", "output_cost_per_image": {price}}}"#);
        entries.push((format!("fal_ai/{size}/{model}"), body));
    }
    let borrowed: Vec<(&str, &str)> = entries
        .iter()
        .map(|(key, body)| (key.as_str(), body.as_str()))
        .collect();
    fixture(&borrowed)
}

fn keyed_fixture() -> Fixture {
    let rows = [
        ("high/1024-x-768", "0.25"),
        ("high/512-x-512", "0.125"),
        ("medium/1024-x-768", "0.125"),
    ];
    priced_fixture("fal-ai/flux/schnell", FLAT_SCHNELL, &rows)
}

#[rstest]
#[case::absent(None, FAL_AI_QUEUE_DEFAULT_BASE)]
#[case::empty(Some(""), FAL_AI_QUEUE_DEFAULT_BASE)]
#[case::configured(Some("https://queue.test/fal"), "https://queue.test/fal")]
fn fal_ai_queue_base_falls_back_for_an_absent_or_empty_setting(
    #[case] configured: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(fal_ai_queue_base(configured), expected);
}

#[test]
fn the_published_constants_match_the_source() {
    assert_eq!(
        FAL_TEXT_TO_IMAGE_DEFAULT_SIZE,
        format!(
            "{}-x-{}",
            DEFAULT_KEYED_DIMENSIONS.0, DEFAULT_KEYED_DIMENSIONS.1
        )
        .as_str()
    );
    assert_eq!(FAL_KEYED_DEFAULT_QUALITY, "high");
    assert_eq!(FAL_PIXELS_PER_MEGAPIXEL, 0x0010_0000);
    assert_eq!(FAL_NAMED_IMAGE_SIZES.len(), 6);
    assert_eq!(FAL_NAMED_IMAGE_SIZES[1], ("square", "512-x-512"));
}

#[rstest]
#[case::plain(Some("1024-x-768"), Some((1024, 768)))]
#[case::zero_width(Some("0-x-768"), None)]
#[case::zero_height(Some("1024-x-0"), None)]
#[case::missing_width(Some("-x-768"), None)]
#[case::missing_separator(Some("1024x768"), None)]
#[case::three_parts(Some("1-x-2-x-3"), None)]
#[case::not_a_number(Some("wide-x-768"), None)]
#[case::absent(None, None)]
fn parse_keyed_dimensions_reads_two_positive_integers(
    #[case] size: Option<&str>,
    #[case] expected: Option<(u64, u64)>,
) {
    assert_eq!(parse_keyed_dimensions(size), expected);
}

#[rstest]
#[case::absent(None, Some(FAL_TEXT_TO_IMAGE_DEFAULT_SIZE))]
#[case::explicit_null(Some(Json::Null), Some(FAL_TEXT_TO_IMAGE_DEFAULT_SIZE))]
#[case::auto(Some(text("auto")), Some(FAL_TEXT_TO_IMAGE_DEFAULT_SIZE))]
#[case::preset(Some(text("square_hd")), Some("1024-x-1024"))]
#[case::square(Some(text("square")), Some("512-x-512"))]
#[case::portrait_4_3(Some(text("portrait_4_3")), Some("768-x-1024"))]
#[case::portrait(Some(text("portrait_16_9")), Some("576-x-1024"))]
#[case::landscape_4_3(Some(text("landscape_4_3")), Some("1024-x-768"))]
#[case::landscape_16_9(Some(text("landscape_16_9")), Some("1024-x-576"))]
#[case::unknown_name(Some(text("huge")), None)]
#[case::padded_name(Some(text(" square")), None)]
#[case::number_value(Some(integer(1024)), None)]
#[case::boolean_value(Some(Json::Bool(true)), None)]
#[case::list_value(Some(Json::List(Vec::new())), None)]
fn keyed_size_reads_a_preset_or_a_dimension_map(
    #[case] image_size: Option<Json>,
    #[case] expected: Option<&str>,
) {
    let params = image_size.map(|value| object(&[("image_size", value)]));
    assert_eq!(keyed_size(params.as_ref()).as_deref(), expected);
}

#[test]
fn keyed_size_formats_a_dimension_map() {
    let valid = object(&[(
        "image_size",
        Json::Object(object(&[
            ("width", integer(1024)),
            ("height", integer(768)),
        ])),
    )]);
    assert_eq!(keyed_size(Some(&valid)).as_deref(), Some("1024-x-768"));
    let zero_width = object(&[(
        "image_size",
        Json::Object(object(&[("width", integer(0)), ("height", integer(768))])),
    )]);
    assert_eq!(keyed_size(Some(&zero_width)), None);
    let float_height = object(&[(
        "image_size",
        Json::Object(object(&[
            ("width", integer(1024)),
            ("height", number("768.0")),
        ])),
    )]);
    assert_eq!(keyed_size(Some(&float_height)), None);
    let boolean_width = object(&[(
        "image_size",
        Json::Object(object(&[
            ("width", Json::Bool(true)),
            ("height", integer(768)),
        ])),
    )]);
    assert_eq!(keyed_size(Some(&boolean_width)), None);
    let missing_height = object(&[(
        "image_size",
        Json::Object(object(&[("width", integer(1024))])),
    )]);
    assert_eq!(keyed_size(Some(&missing_height)), None);
}

#[test]
fn image_dimensions_requires_two_positive_exact_integers() {
    let valid = object(&[("width", integer(1024)), ("height", integer(768))]);
    assert_eq!(image_dimensions(Some(&valid)), Some((1024, 768)));
    let boolean_width = object(&[("width", Json::Bool(true)), ("height", integer(768))]);
    assert_eq!(image_dimensions(Some(&boolean_width)), None);
    let float_width = object(&[("width", number("1024.0")), ("height", integer(768))]);
    assert_eq!(image_dimensions(Some(&float_width)), None);
    let negative_width = object(&[("width", number("-5")), ("height", integer(768))]);
    assert_eq!(image_dimensions(Some(&negative_width)), None);
    let zero_height = object(&[("width", integer(1024)), ("height", integer(0))]);
    assert_eq!(image_dimensions(Some(&zero_height)), None);
    let text_width = object(&[("width", text("1024")), ("height", integer(768))]);
    assert_eq!(image_dimensions(Some(&text_width)), None);
    let only_width = object(&[("width", integer(1024))]);
    assert_eq!(image_dimensions(Some(&only_width)), None);
    assert_eq!(image_dimensions(None), None);
}

#[rstest]
#[case::absent(None, FAL_KEYED_DEFAULT_QUALITY)]
#[case::named(Some(text("medium")), "medium")]
#[case::auto(Some(text("auto")), FAL_KEYED_DEFAULT_QUALITY)]
#[case::number(Some(integer(5)), FAL_KEYED_DEFAULT_QUALITY)]
#[case::boolean(Some(Json::Bool(true)), FAL_KEYED_DEFAULT_QUALITY)]
#[case::explicit_null(Some(Json::Null), FAL_KEYED_DEFAULT_QUALITY)]
fn keyed_quality_is_a_string_other_than_auto(
    #[case] quality: Option<Json>,
    #[case] expected: &str,
) {
    let params = quality.map(|value| object(&[("quality", value)]));
    assert_eq!(keyed_quality(params.as_ref()), expected);
}

#[rstest]
#[case::absent(None, None)]
#[case::explicit_null(Some(Json::Null), None)]
#[case::boolean(Some(Json::Bool(true)), None)]
#[case::float(Some(number("1024.0")), None)]
#[case::list(Some(Json::List(Vec::new())), None)]
#[case::integer(Some(integer(1024)), Some("1024"))]
#[case::negative_integer(Some(number("-5")), Some("-5"))]
#[case::word(Some(text("1K")), Some("1K"))]
fn resolution_key_reads_integers_and_strings(
    #[case] resolution: Option<Json>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(resolution_key(resolution.as_ref()).as_deref(), expected);
}

fn request<'request>(
    model: &'request str,
    images: &'request [Option<&'request IndexMap<String, Json>>],
    params: Option<&'request IndexMap<String, Json>>,
    info: Option<&'request PriceRecord>,
) -> FalCostRequest<'request> {
    FalCostRequest {
        model,
        image_response: FalImageResponse::Image(images),
        optional_params: params,
        model_info: info,
    }
}

#[test]
fn returned_dimensions_beat_the_requested_size() {
    let fixture = keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let params = object(&[("image_size", text("square"))]);
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, Some(&params), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the keyed row prices the image");
    assert_eq!(cost, decimal("0.25"));
}

#[test]
fn invalid_returned_dimensions_fall_back_to_the_requested_size() {
    let fixture = keyed_fixture();
    let image = object(&[("width", Json::Bool(true)), ("height", integer(768))]);
    let images = [Some(&image)];
    let params = object(&[(
        "image_size",
        Json::Object(object(&[("width", integer(512)), ("height", integer(512))])),
    )]);
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, Some(&params), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the requested size picks the row");
    assert_eq!(cost, decimal("0.125"));
}

#[test]
fn an_unknown_size_name_falls_back_to_the_default_dimensions() {
    let fixture = keyed_fixture();
    let image = object(&[]);
    let images = [Some(&image)];
    let params = object(&[("image_size", text("huge"))]);
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, Some(&params), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the default dimensions pick the row");
    assert_eq!(cost, decimal("0.25"));
}

#[rstest]
#[case::medium("medium", "0.125")]
#[case::automatic("auto", "0.25")]
fn a_named_quality_selects_its_own_keyed_rows(#[case] quality: &str, #[case] expected: &str) {
    let fixture = keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let params = object(&[("quality", text(quality))]);
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, Some(&params), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the quality row prices the image");
    assert_eq!(cost, decimal(expected));
}

#[rstest]
#[case::numeric(Some(5))]
#[case::absent(None)]
fn a_non_string_quality_uses_the_default_keyed_rows(#[case] quality: Option<u64>) {
    let fixture = keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let params = quality.map(|value| object(&[("quality", integer(value))]));
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, params.as_ref(), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the default quality row prices the image");
    assert_eq!(cost, decimal("0.25"));
}

fn tie_fixture() -> Fixture {
    let rows = [("high/1000-x-1000", "0.2"), ("high/900-x-1000", "0.1")];
    priced_fixture("fal-ai/tie", SKIP_FLAT, &rows)
}

#[test]
fn the_closest_area_wins_and_a_tie_goes_to_the_smaller_area() {
    let fixture = tie_fixture();
    let image = object(&[]);
    let images = [Some(&image)];
    let tied = object(&[(
        "image_size",
        Json::Object(object(&[
            ("width", integer(950)),
            ("height", integer(1000)),
        ])),
    )]);
    let closer = object(&[(
        "image_size",
        Json::Object(object(&[
            ("width", integer(1150)),
            ("height", integer(1000)),
        ])),
    )]);
    let tied_cost = cost_calculator(
        &request("fal_ai/fal-ai/tie", &images, Some(&tied), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the smaller tied area prices the image");
    assert_eq!(tied_cost, decimal("0.1"));
    let closer_cost = cost_calculator(
        &request("fal_ai/fal-ai/tie", &images, Some(&closer), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the closer area prices the image");
    assert_eq!(closer_cost, decimal("0.2"));
}

#[test]
fn a_keyed_row_without_a_price_or_a_parsable_size_is_skipped() {
    let fixture = fixture(&[
        ("fal_ai/fal-ai/skip", SKIP_FLAT),
        (
            "fal_ai/high/1024-x-768/fal-ai/skip",
            r#"{"litellm_provider": "fal_ai"}"#,
        ),
        (
            "fal_ai/high/wide-x-768/fal-ai/skip",
            r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.01}"#,
        ),
    ]);
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let cost = cost_calculator(
        &request("fal_ai/fal-ai/skip", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the flat price prices the image");
    assert_eq!(cost, decimal("0.75"));
}

#[test]
fn a_keyed_row_the_lookup_does_not_know_is_skipped() {
    let fixture = Fixture {
        lookup: lookup_from(&[("fal_ai/fal-ai/skip", SKIP_FLAT)]),
        catalog: fixture(&[
            ("fal_ai/fal-ai/skip", SKIP_FLAT),
            (
                "fal_ai/high/1024-x-768/fal-ai/skip",
                r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.01}"#,
            ),
        ])
        .catalog,
    };
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let cost = cost_calculator(
        &request("fal_ai/fal-ai/skip", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the flat price prices the image");
    assert_eq!(cost, decimal("0.75"));
}

#[test]
fn equal_areas_keep_the_catalog_order() {
    let fixture = fixture(&[
        (
            "fal_ai/high/1000-x-1000/fal-ai/square",
            r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.1}"#,
        ),
        (
            "fal_ai/high/500-x-2000/fal-ai/square",
            r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.2}"#,
        ),
    ]);
    let image = object(&[("width", integer(1000)), ("height", integer(1000))]);
    let images = [Some(&image)];
    let cost = cost_calculator(
        &request("fal_ai/fal-ai/square", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the first equal-area row prices the image");
    assert_eq!(cost, decimal("0.1"));
}

fn pixel_fixture() -> Fixture {
    fixture(&[(
        "fal_ai/fal-ai/pixel",
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.75, "output_cost_per_pixel": 0.000002}"#,
    )])
}

#[rstest]
#[case::exact(1024, 1024, "2.097152")]
#[case::rounded(1024, 1025, "4.194304")]
#[case::quarter(512, 512, "2.097152")]
fn pixel_pricing_rounds_up_to_whole_megapixels(
    #[case] width: u64,
    #[case] height: u64,
    #[case] expected: &str,
) {
    let fixture = pixel_fixture();
    let image = object(&[("width", integer(width)), ("height", integer(height))]);
    let images = [Some(&image)];
    let cost = cost_calculator(
        &request("fal_ai/fal-ai/pixel", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the pixel rate prices the image");
    assert_eq!(cost, decimal(expected));
}

#[test]
fn a_flat_price_applies_when_the_returned_dimensions_are_missing() {
    let fixture = pixel_fixture();
    let image = object(&[("width", Json::Bool(true)), ("height", integer(1024))]);
    let images = [Some(&image)];
    let cost = cost_calculator(
        &request("fal_ai/fal-ai/pixel", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the flat price applies");
    assert_eq!(cost, decimal("0.75"));
}

#[test]
fn a_missing_pixel_rate_uses_the_flat_price() {
    let fixture = fixture(&[(
        "fal_ai/fal-ai/flat",
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.75}"#,
    )]);
    let image = object(&[("width", integer(1024)), ("height", integer(1024))]);
    let images = [Some(&image)];
    let cost = cost_calculator(
        &request("fal_ai/fal-ai/flat", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the flat price applies");
    assert_eq!(cost, decimal("0.75"));
}

#[test]
fn a_deployment_image_price_wins_even_when_it_is_zero() {
    let fixture = keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    let zero = record(&[("output_cost_per_image", "0")]);
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, None, Some(&zero)),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the deployment price prices the images");
    assert_eq!(cost, Decimal::ZERO);
    let priced = record(&[("output_cost_per_image", "0.25")]);
    let priced_cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, None, Some(&priced)),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the deployment price prices the images");
    assert_eq!(priced_cost, decimal("0.5"));
}

#[test]
fn a_boolean_deployment_price_reads_as_one() {
    let fixture = keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    let flag = record(&[("output_cost_per_image", "true")]);
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, None, Some(&flag)),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the boolean deployment price prices the images");
    assert_eq!(cost, decimal("2"));
}

#[rstest]
#[case::token_rate("output_cost_per_token", "1")]
#[case::malformed_image_price("output_cost_per_image", r#""high""#)]
fn a_deployment_without_an_image_price_keeps_the_catalog_price(
    #[case] key: &str,
    #[case] value: &str,
) {
    let fixture = keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let info = record(&[(key, value)]);
    let cost = cost_calculator(
        &request(FLUX_SCHNELL, &images, None, Some(&info)),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the keyed row prices the image");
    assert_eq!(cost, decimal("0.25"));
}

#[test]
fn a_deployment_pixel_rate_prices_a_model_the_catalog_lacks() {
    let fixture = fixture(&[]);
    let image = object(&[("width", integer(1024)), ("height", integer(1025))]);
    let images = [Some(&image)];
    let info = record(&[("output_cost_per_pixel", "0.000002")]);
    let cost = cost_calculator(
        &request("fal_ai/fal-ai/unlisted", &images, None, Some(&info)),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the deployment pixel rate prices the image");
    assert_eq!(cost, decimal("4.194304"));
}

fn dev_fixture() -> Fixture {
    fixture(&[(FLUX_DEV, FLAT_DEV)])
}

fn dev_resolution_cost(params: &IndexMap<String, Json>) -> Decimal {
    let fixture = dev_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    cost_calculator(
        &request(FLUX_DEV, &images, Some(params), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the resolution price prices the images")
}

#[test]
fn a_numeric_resolution_price_beats_the_flat_price() {
    assert_eq!(
        dev_resolution_cost(&object(&[("resolution", integer(1024))])),
        decimal("0.25")
    );
}

#[test]
fn a_named_resolution_price_beats_the_flat_price() {
    assert_eq!(
        dev_resolution_cost(&object(&[("resolution", text("1024"))])),
        decimal("0.25")
    );
}

#[test]
fn an_unknown_resolution_name_falls_back_to_the_flat_price() {
    assert_eq!(
        dev_resolution_cost(&object(&[("resolution", text("1024p"))])),
        decimal("1")
    );
}

#[test]
fn a_boolean_resolution_falls_back_to_the_flat_price() {
    assert_eq!(
        dev_resolution_cost(&object(&[("resolution", Json::Bool(true))])),
        decimal("1")
    );
}

#[test]
fn a_float_resolution_falls_back_to_the_flat_price() {
    assert_eq!(
        dev_resolution_cost(&object(&[("resolution", number("1024.0"))])),
        decimal("1")
    );
}

#[test]
fn a_lowercase_resolution_key_falls_back_to_the_flat_price() {
    assert_eq!(
        dev_resolution_cost(&object(&[("resolution", text("1k"))])),
        decimal("1")
    );
}

#[test]
fn a_zero_resolution_price_is_used_as_zero() {
    let fixture = fixture(&[(
        FLUX_DEV,
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.5, "output_cost_per_image_1024": 0}"#,
    )]);
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    let params = object(&[("resolution", integer(1024))]);
    let cost = cost_calculator(
        &request(FLUX_DEV, &images, Some(&params), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the zero resolution price prices the images");
    assert_eq!(cost, Decimal::ZERO);
}

fn dev_keyed_fixture() -> Fixture {
    let rows = [("high/1024-x-768", "0.3")];
    priced_fixture("fal-ai/flux/dev", FLAT_DEV, &rows)
}

#[test]
fn a_zero_deployment_price_outranks_a_matching_resolution_price() {
    let fixture = dev_keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    let params = object(&[("resolution", integer(1024))]);
    let zero = record(&[("output_cost_per_image", "0")]);
    let cost = cost_calculator(
        &request(FLUX_DEV, &images, Some(&params), Some(&zero)),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the deployment price prices the images");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_resolution_price_outranks_the_keyed_rows_of_the_same_model() {
    let fixture = dev_keyed_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    let resolved = object(&[("resolution", integer(1024))]);
    let cost = cost_calculator(
        &request(FLUX_DEV, &images, Some(&resolved), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the resolution price prices the images");
    assert_eq!(cost, decimal("0.25"));
    let unresolved = object(&[("resolution", text("1k"))]);
    let keyed = cost_calculator(
        &request(FLUX_DEV, &images, Some(&unresolved), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the keyed rows price the images");
    assert_eq!(keyed, decimal("0.6"));
}

#[test]
fn a_boolean_resolution_price_reads_as_one() {
    let fixture = fixture(&[]);
    let lookup = lookup_from(&[(
        FLUX_DEV,
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image_1024": true}"#,
    )]);
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    let params = object(&[("resolution", integer(1024))]);
    let cost = cost_calculator(
        &request(FLUX_DEV, &images, Some(&params), None),
        &fixture.catalog,
        &lookup,
    )
    .expect("the boolean resolution price prices the images");
    assert_eq!(cost, decimal("2"));
}

#[test]
fn a_price_decimal_cannot_hold_is_a_typed_error() {
    let fixture = fixture(&[]);
    let lookup = lookup_from(&[(
        FLUX_DEV,
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image_1024": 1e30}"#,
    )]);
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let params = object(&[("resolution", integer(1024))]);
    let result = cost_calculator(
        &request(FLUX_DEV, &images, Some(&params), None),
        &fixture.catalog,
        &lookup,
    );
    assert!(matches!(result, Err(FalPricingError::ImageCost(_))));
}

#[test]
fn a_non_numeric_resolution_price_is_ignored() {
    let fixture = fixture(&[]);
    let lookup = lookup_from(&[(
        FLUX_DEV,
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.5, "output_cost_per_image_1024": "0.125"}"#,
    )]);
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image), Some(&image)];
    let params = object(&[("resolution", integer(1024))]);
    let cost = cost_calculator(
        &request(FLUX_DEV, &images, Some(&params), None),
        &fixture.catalog,
        &lookup,
    )
    .expect("the flat price prices the images");
    assert_eq!(cost, decimal("1"));
}

#[test]
fn an_unprefixed_model_reads_the_same_catalog_entry() {
    let fixture = dev_fixture();
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let params = object(&[("resolution", integer(1024))]);
    let cost = cost_calculator(
        &request("fal-ai/flux/dev", &images, Some(&params), None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("the resolution price prices the image");
    assert_eq!(cost, decimal("0.125"));
}

#[test]
fn an_empty_response_costs_zero_without_resolving_the_model() {
    let fixture = fixture(&[]);
    let images: [Option<&IndexMap<String, Json>>; 0] = [];
    let cost = cost_calculator(
        &request("fal_ai/unknown/model", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    )
    .expect("an empty response costs zero");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_missing_model_price_is_an_error_for_a_returned_image() {
    let fixture = fixture(&[]);
    let image = object(&[("width", integer(1024)), ("height", integer(768))]);
    let images = [Some(&image)];
    let result = cost_calculator(
        &request("fal_ai/unknown/model", &images, None, None),
        &fixture.catalog,
        &fixture.lookup,
    );
    assert!(matches!(
        result,
        Err(FalPricingError::ImageCost(
            ImageCostError::UnknownModel { .. }
        ))
    ));
}

#[test]
fn a_non_image_response_is_rejected() {
    let fixture = fixture(&[]);
    let request = FalCostRequest {
        model: FLUX_SCHNELL,
        image_response: FalImageResponse::Other,
        optional_params: None,
        model_info: None,
    };
    assert!(matches!(
        cost_calculator(&request, &fixture.catalog, &fixture.lookup),
        Err(FalPricingError::NotImageResponse)
    ));
}
