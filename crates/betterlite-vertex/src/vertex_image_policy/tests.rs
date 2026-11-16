#![cfg(test)]

use betterlite_models::decisions::Extra;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::usage::ImageUsageDetails;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageUsage};
use rstest::rstest;
use rust_decimal::Decimal;

use super::config::{EditConfig, GenerationConfig, edit_config, generation_config};
use super::cost::{
    BilledUsage, EditResponse, GeneratedImages, ImagePricing, edit_cost, generation_cost,
};
use super::error::PredictionError;
use super::prediction::{is_image_generation_response, process_image_generation_response};
use crate::vertex_imagen_generation::response::ImagenGenerationResponse;

fn record(source: &str) -> PriceRecord {
    serde_json::from_str(source).expect("price record source")
}

fn reply(source: &str) -> Extra {
    serde_json::from_str(source).expect("reply source")
}

fn cents(amount: i64) -> Decimal {
    Decimal::new(amount, 2)
}

// the shared pricing operations, answering from fixed numbers so each policy branch shows in the total
struct Pricing {
    known: bool,
    catalog: PriceRecord,
    search_per_request: Decimal,
    token_cost: Option<Decimal>,
}

impl Pricing {
    fn new(catalog: &str) -> Self {
        Self {
            known: true,
            catalog: record(catalog),
            search_per_request: cents(5),
            token_cost: None,
        }
    }
}

impl ImagePricing for Pricing {
    fn model_info(
        &self,
        model: &str,
        provider: &str,
        supplied: Option<&PriceRecord>,
    ) -> Result<PriceRecord, ImageCostError> {
        if !self.known || provider != "vertex_ai" {
            return Err(ImageCostError::UnknownModel {
                model: model.to_owned(),
            });
        }
        Ok(supplied.map_or_else(|| self.catalog.clone(), Clone::clone))
    }

    fn web_search_cost(
        &self,
        provider: &str,
        web_search_requests: Option<i64>,
        _model_info: &PriceRecord,
    ) -> Decimal {
        assert_eq!(provider, "vertex_ai");
        self.search_per_request
            .checked_mul(Decimal::from(web_search_requests.unwrap_or(0)))
            .expect("the product fits")
    }

    fn usage_cost(
        &self,
        _model: &str,
        _provider: &str,
        _usage: &ImageUsageDetails,
        _model_info: &PriceRecord,
    ) -> Result<Option<Decimal>, ImageCostError> {
        Ok(self.token_cost)
    }
}

const fn billed(tokens: &ImageUsageDetails, web_search_requests: Option<i64>) -> BilledUsage<'_> {
    BilledUsage {
        tokens,
        web_search_requests,
    }
}

#[rstest]
#[case::gemini_image("gemini-2.5-flash-image", GenerationConfig::Gemini)]
#[case::prefixed_gemini("vertex_ai/gemini-3-pro-image", GenerationConfig::Gemini)]
#[case::imagen("imagen-3.0-generate-002", GenerationConfig::Imagen)]
#[case::legacy_imagegeneration("imagegeneration@006", GenerationConfig::Imagen)]
#[case::partner_route("claude-3-5-sonnet", GenerationConfig::Imagen)]
#[case::model_garden_route("openai/gpt-oss", GenerationConfig::Imagen)]
#[case::empty_model("", GenerationConfig::Imagen)]
fn generation_selects_gemini_only_for_the_gemini_route(
    #[case] model: &str,
    #[case] expected: GenerationConfig,
) {
    assert_eq!(generation_config(model), expected);
}

#[rstest]
#[case::gemini_image("gemini-2.5-flash-image", EditConfig::Gemini)]
#[case::prefixed_gemini("vertex_ai/gemini-3-pro-image", EditConfig::Gemini)]
#[case::imagen("imagen-3.0-capability-001", EditConfig::Imagen)]
#[case::legacy_imagegeneration("imagegeneration@006", EditConfig::Imagen)]
#[case::partner_route("claude-3-5-sonnet", EditConfig::Imagen)]
#[case::model_garden_route("openai/gpt-oss", EditConfig::Imagen)]
#[case::empty_model("", EditConfig::Imagen)]
fn edit_selects_gemini_only_for_the_gemini_route(
    #[case] model: &str,
    #[case] expected: EditConfig,
) {
    assert_eq!(edit_config(model), expected);
}

#[test]
fn a_zero_token_price_suppresses_the_per_image_fallback() {
    let mut pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    pricing.token_cost = Some(Decimal::ZERO);
    let usage = ImageUsageDetails::default();
    let response = GeneratedImages {
        image_count: 3,
        usage: Some(billed(&usage, None)),
    };
    let cost = generation_cost(&pricing, "imagen", None, &response).expect("priced");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_token_price_replaces_the_per_image_price_and_search_is_added() {
    let mut pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    pricing.token_cost = Some(cents(30));
    let usage = ImageUsageDetails::default();
    let response = GeneratedImages {
        image_count: 3,
        usage: Some(billed(&usage, Some(2))),
    };
    let cost = generation_cost(&pricing, "gemini", None, &response).expect("priced");
    assert_eq!(cost, cents(40));
}

#[test]
fn missing_token_pricing_bills_each_returned_image_and_adds_search() {
    let pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    let usage = ImageUsageDetails::default();
    let response = GeneratedImages {
        image_count: 3,
        usage: Some(billed(&usage, Some(1))),
    };
    let cost = generation_cost(&pricing, "gemini", None, &response).expect("priced");
    assert_eq!(cost, cents(17));
}

#[test]
fn a_response_without_usage_bills_images_alone() {
    let mut pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    // a usage-less response never asks for token cost, so this answer must stay unused
    pricing.token_cost = Some(cents(99));
    let response = GeneratedImages {
        image_count: 2,
        usage: None,
    };
    let cost = generation_cost(&pricing, "imagen", None, &response).expect("priced");
    assert_eq!(cost, cents(8));
}

#[test]
fn search_is_charged_when_no_image_came_back() {
    let pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    let usage = ImageUsageDetails::default();
    let response = GeneratedImages {
        image_count: 0,
        usage: Some(billed(&usage, Some(2))),
    };
    let cost = generation_cost(&pricing, "gemini", None, &response).expect("priced");
    assert_eq!(cost, cents(10));
}

#[rstest]
#[case::null_price(r#"{"output_cost_per_image": null}"#)]
#[case::absent_price("{}")]
fn a_missing_per_image_price_costs_nothing(#[case] catalog: &str) {
    let pricing = Pricing::new(catalog);
    let response = GeneratedImages {
        image_count: 4,
        usage: None,
    };
    let cost = generation_cost(&pricing, "imagen", None, &response).expect("priced");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn a_supplied_price_table_reaches_the_resolver() {
    let pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    let supplied = record(r#"{"output_cost_per_image": 0.10}"#);
    let response = GeneratedImages {
        image_count: 2,
        usage: None,
    };
    let cost = generation_cost(&pricing, "imagen", Some(&supplied), &response).expect("priced");
    assert_eq!(cost, cents(20));
}

#[test]
fn generation_cost_surfaces_a_failed_model_lookup() {
    let mut pricing = Pricing::new("{}");
    pricing.known = false;
    let response = GeneratedImages {
        image_count: 1,
        usage: None,
    };
    let error = generation_cost(&pricing, "mystery", None, &response).expect_err("unknown model");
    assert!(matches!(error, ImageCostError::UnknownModel { model } if model == "mystery"));
}

#[test]
fn generation_cost_names_a_price_of_the_wrong_type() {
    let pricing = Pricing::new(r#"{"output_cost_per_image": "cheap"}"#);
    let response = GeneratedImages {
        image_count: 1,
        usage: None,
    };
    let error = generation_cost(&pricing, "imagen", None, &response).expect_err("wrong type");
    assert!(
        matches!(error, ImageCostError::WrongFieldType { field } if field == "output_cost_per_image")
    );
}

#[test]
fn edit_billing_ignores_usage_tokens_and_search() {
    let mut pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    pricing.token_cost = Some(cents(99));
    let cost = edit_cost(
        &pricing,
        "imagen-edit",
        EditResponse::Image { image_count: 3 },
    )
    .expect("priced");
    assert_eq!(cost, cents(12));
}

#[test]
fn edit_billing_of_no_images_is_zero() {
    let pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    let cost = edit_cost(
        &pricing,
        "imagen-edit",
        EditResponse::Image { image_count: 0 },
    )
    .expect("priced");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn edit_billing_without_a_price_is_zero() {
    let pricing = Pricing::new(r#"{"output_cost_per_image": null}"#);
    let cost = edit_cost(
        &pricing,
        "imagen-edit",
        EditResponse::Image { image_count: 2 },
    )
    .expect("priced");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn edit_billing_refuses_a_response_that_is_not_an_image_response() {
    let pricing = Pricing::new(r#"{"output_cost_per_image": 0.04}"#);
    let error = edit_cost(&pricing, "imagen-edit", EditResponse::Other).expect_err("not images");
    assert!(matches!(error, ImageCostError::NotImageResponse));
}

#[test]
fn edit_billing_resolves_model_info_before_the_response_type_check() {
    let mut pricing = Pricing::new("{}");
    pricing.known = false;
    let error = edit_cost(&pricing, "mystery", EditResponse::Other).expect_err("unknown model");
    assert!(matches!(error, ImageCostError::UnknownModel { .. }));
}

#[rstest]
#[case::first_has_the_key(r#"{"predictions": [{"bytesBase64Encoded": "AAA"}]}"#, true)]
#[case::null_value_still_counts(r#"{"predictions": [{"bytesBase64Encoded": null}]}"#, true)]
#[case::first_lacks_the_key(
    r#"{"predictions": [{"raiFilteredReason": "x"}, {"bytesBase64Encoded": "AAA"}]}"#,
    false
)]
#[case::later_predictions_are_not_scanned(
    r#"{"predictions": [{"bytesBase64Encoded": "AAA"}, {"raiFilteredReason": "x"}]}"#,
    true
)]
#[case::no_predictions_key(r#"{"other": []}"#, false)]
fn detection_reads_the_first_prediction_only(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(
        is_image_generation_response(&reply(source)).expect("detected"),
        expected
    );
}

#[test]
fn detection_of_an_empty_prediction_list_is_its_own_error() {
    let error = is_image_generation_response(&reply(r#"{"predictions": []}"#)).expect_err("empty");
    assert!(matches!(error, PredictionError::EmptyPredictions));
}

#[rstest]
#[case::text(r#"{"predictions": "AAA"}"#, "predictions", "a string")]
#[case::number(r#"{"predictions": 4}"#, "predictions", "a number")]
#[case::boolean(r#"{"predictions": true}"#, "predictions", "a boolean")]
#[case::null(r#"{"predictions": null}"#, "predictions", "null")]
#[case::object(r#"{"predictions": {}}"#, "predictions", "an object")]
#[case::text_member(r#"{"predictions": ["AAA"]}"#, "predictions[0]", "a string")]
#[case::list_member(r#"{"predictions": [[]]}"#, "predictions[0]", "a list")]
fn detection_names_a_malformed_predictions_field(
    #[case] source: &str,
    #[case] path: &str,
    #[case] kind: &str,
) {
    let error = is_image_generation_response(&reply(source)).expect_err("malformed");
    assert!(
        matches!(error, PredictionError::Shape { path: found, found: seen } if found == path && seen == kind)
    );
}

fn image(b64_json: Option<&str>) -> ImageData {
    ImageData {
        b64_json: b64_json.map(str::to_owned),
        revised_prompt: None,
        url: None,
    }
}

#[test]
fn conversion_replaces_existing_images_with_one_per_prediction() {
    let mut response = ImagenGenerationResponse {
        data: vec![image(Some("OLD"))],
        usage: Some(ImageUsage::default()),
    };
    let body = reply(
        r#"{"predictions": [{"bytesBase64Encoded": "AAA"}, {"bytesBase64Encoded": null, "mimeType": "image/png"}]}"#,
    );
    process_image_generation_response(&body, &mut response).expect("converted");
    assert_eq!(response.data, vec![image(Some("AAA")), image(None)]);
    assert_eq!(response.usage, Some(ImageUsage::default()));
}

#[test]
fn conversion_of_an_empty_prediction_list_clears_the_images() {
    let mut response = ImagenGenerationResponse {
        data: vec![image(Some("OLD"))],
        usage: None,
    };
    process_image_generation_response(&reply(r#"{"predictions": []}"#), &mut response)
        .expect("converted");
    assert!(response.data.is_empty());
}

#[test]
fn conversion_without_predictions_is_its_own_error_and_keeps_the_images() {
    let mut response = ImagenGenerationResponse {
        data: vec![image(Some("OLD"))],
        usage: None,
    };
    let error = process_image_generation_response(&reply(r#"{"other": 1}"#), &mut response)
        .expect_err("no predictions");
    assert!(matches!(error, PredictionError::MissingPredictions));
    assert_eq!(response.data, vec![image(Some("OLD"))]);
}

#[test]
fn conversion_names_the_prediction_missing_the_image_key_and_keeps_the_images() {
    let mut response = ImagenGenerationResponse {
        data: vec![image(Some("OLD"))],
        usage: None,
    };
    let body = reply(
        r#"{"predictions": [{"bytesBase64Encoded": "AAA"}, {"raiFilteredReason": "blocked"}]}"#,
    );
    let error = process_image_generation_response(&body, &mut response).expect_err("missing key");
    assert!(matches!(
        error,
        PredictionError::MissingImageKey { index: 1 }
    ));
    assert_eq!(response.data, vec![image(Some("OLD"))]);
}

#[test]
fn conversion_refuses_an_image_value_that_is_not_text() {
    let mut response = ImagenGenerationResponse::default();
    let body = reply(r#"{"predictions": [{"bytesBase64Encoded": 7}]}"#);
    let error = process_image_generation_response(&body, &mut response).expect_err("not text");
    assert!(matches!(error, PredictionError::ImageType { index: 0 }));
}

#[test]
fn conversion_names_a_malformed_predictions_field() {
    let mut response = ImagenGenerationResponse::default();
    let error = process_image_generation_response(&reply(r#"{"predictions": 4}"#), &mut response)
        .expect_err("not a list");
    assert!(matches!(error, PredictionError::Shape { .. }));
}
