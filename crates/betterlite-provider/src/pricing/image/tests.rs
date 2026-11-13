#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::cost::image_dispatch::error::ImageDispatchError;
use betterlite_core::cost::image_dispatch::options::{OptionalParams, SizePattern};
use betterlite_core::cost::image_dispatch::policy::{ImageResponseView, ImageRoute, PricedImage};
use betterlite_core::cost::image_dispatch::route::{ImageCall, route_image_cost};
use betterlite_models::decisions::Json;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::token_rates::ModelInfo;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;

use super::ProviderImagePricing;
use super::error::ImagePolicyError;

struct Catalog(BTreeMap<String, PriceRecord>);

impl PriceLookup for Catalog {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.0.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

fn record(text: &str) -> PriceRecord {
    serde_json::from_str(text).unwrap()
}

fn deployment(price: &str) -> ModelInfo {
    serde_json::from_str(&format!(r#"{{"output_cost_per_image":"{price}"}}"#)).unwrap()
}

fn billing(image_count: i64) -> ImageBilling {
    ImageBilling {
        image_count,
        size: None,
        quality: None,
        usage: None,
    }
}

fn image_call<'call>(
    provider: &'call str,
    model: &'call str,
    response: ImageResponseView<'call>,
) -> ImageCall<'call> {
    ImageCall {
        model,
        custom_llm_provider: Some(provider),
        quality: None,
        image_count: None,
        size: None,
        optional_params: None,
        call_type: None,
        model_info: None,
        response,
    }
}

fn price(
    call: &ImageCall<'_>,
    catalog: &Catalog,
) -> Result<Decimal, ImageDispatchError<ImagePolicyError>> {
    let database = TimeZoneDatabase::from_env();
    let policy = ProviderImagePricing {
        lookup: catalog,
        database: &database,
        moment: DateTime::<Utc>::UNIX_EPOCH,
    };
    route_image_cost(&policy, &size_pattern(), call)
}

fn size_pattern() -> SizePattern {
    SizePattern::new().unwrap()
}

fn empty() -> Catalog {
    Catalog(BTreeMap::new())
}

#[rstest]
#[case::default_returned_count("stability", "sdxl", None, "0.08")]
#[case::default_explicit_count("stability", "sdxl", Some(5), "0.20")]
#[case::default_explicit_zero("stability", "sdxl", Some(0), "0.00")]
#[case::azure_ai_returned_count("azure_ai", "flux", None, "0.08")]
#[case::azure_ai_explicit_count("azure_ai", "flux", Some(5), "0.20")]
fn the_resolved_count_is_billed_at_the_deployment_price(
    #[case] provider: &str,
    #[case] model: &str,
    #[case] count: Option<i64>,
    #[case] expected: &str,
) {
    let info = deployment("0.04");
    let response = billing(2);
    let mut call = image_call(provider, model, ImageResponseView::Image(&response));
    call.model_info = Some(&info);
    call.image_count = count;
    assert_eq!(
        price(&call, &empty()).unwrap(),
        expected.parse::<Decimal>().unwrap()
    );
}

#[test]
fn the_default_calculator_without_any_price_table_names_the_unknown_model() {
    let response = billing(1);
    let call = image_call("stability", "sdxl", ImageResponseView::Image(&response));
    assert!(matches!(
        price(&call, &empty()),
        Err(ImageDispatchError::Policy(ImagePolicyError::Image(
            ImageCostError::UnknownModel { .. }
        )))
    ));
}

#[rstest]
#[case::lower("gpt-image-1")]
#[case::mixed_case("GPT-Image-1.5")]
fn gpt_image_models_use_the_openai_calculator_over_the_deployment_prices(#[case] model: &str) {
    let info = deployment("0.04");
    let response = billing(2);
    let mut call = image_call("openai", model, ImageResponseView::Image(&response));
    call.model_info = Some(&info);
    assert_eq!(price(&call, &empty()).unwrap(), Decimal::new(8, 2));
}

#[test]
fn a_gpt_image_model_without_deployment_prices_costs_zero() {
    let response = billing(2);
    let call = image_call("azure", "gpt-image-1", ImageResponseView::Image(&response));
    assert_eq!(price(&call, &empty()).unwrap(), Decimal::ZERO);
}

#[test]
fn a_gpt_image_route_rejects_a_response_that_is_not_an_image_response() {
    let call = image_call("openai", "gpt-image-1", ImageResponseView::Other);
    assert!(matches!(
        price(&call, &empty()),
        Err(ImageDispatchError::Policy(ImagePolicyError::Image(
            ImageCostError::NotImageResponse
        )))
    ));
}

#[test]
fn azure_ai_rejects_a_response_that_is_not_an_image_response() {
    let info = deployment("0.04");
    let mut call = image_call("azure_ai", "flux", ImageResponseView::Other);
    call.model_info = Some(&info);
    assert!(matches!(
        price(&call, &empty()),
        Err(ImageDispatchError::Policy(ImagePolicyError::Image(
            ImageCostError::NotImageResponse
        )))
    ));
}

#[rstest]
#[case::dimensions_from_the_options(Some(4), Some(3), "0.000012")]
#[case::a_missing_height_keeps_the_size(Some(4), None, "1.048576")]
#[case::no_dimensions_keep_the_size(None, None, "1.048576")]
fn azure_ai_prices_pixels_from_the_width_and_height_options(
    #[case] width: Option<i64>,
    #[case] height: Option<i64>,
    #[case] expected: &str,
) {
    let info: ModelInfo = serde_json::from_str(r#"{"input_cost_per_pixel":"0.000001"}"#).unwrap();
    let response = billing(1);
    let options: OptionalParams = [("width", width), ("height", height)]
        .into_iter()
        .filter_map(|(key, value)| {
            value.map(|dimension| (key.to_owned(), Json::Number(Number::from(dimension))))
        })
        .collect();
    let mut call = image_call("azure_ai", "flux", ImageResponseView::Image(&response));
    call.model_info = Some(&info);
    call.optional_params = Some(&options);
    assert_eq!(
        price(&call, &empty()).unwrap(),
        expected.parse::<Decimal>().unwrap()
    );
}

#[test]
fn bedrock_prices_the_titan_family_from_the_catalog_entry() {
    let catalog = Catalog(BTreeMap::from([(
        "amazon.titan-image-generator-v1".to_owned(),
        record(r#"{"output_cost_per_image":0.01}"#),
    )]));
    let response = billing(3);
    let call = image_call(
        "bedrock",
        "amazon.titan-image-generator-v1",
        ImageResponseView::Image(&response),
    );
    assert_eq!(price(&call, &catalog).unwrap(), Decimal::new(3, 2));
}

#[rstest]
#[case::default_steps(None, "1024-x-1024/50-steps/stability.sdxl")]
#[case::many_steps(Some(80), "1024-x-1024/max-steps/stability.sdxl")]
fn bedrock_stability_v1_folds_the_requested_steps_into_the_catalog_key(
    #[case] steps: Option<i64>,
    #[case] key: &str,
) {
    let catalog = Catalog(BTreeMap::from([(
        key.to_owned(),
        record(r#"{"output_cost_per_image":0.02}"#),
    )]));
    let response = billing(2);
    let mut options = OptionalParams::new();
    if let Some(count) = steps {
        options.insert("steps".to_owned(), Json::Number(Number::from(count)));
    }
    let mut call = image_call(
        "bedrock",
        "stability.sdxl",
        ImageResponseView::Image(&response),
    );
    call.optional_params = Some(&options);
    assert_eq!(price(&call, &catalog).unwrap(), Decimal::new(4, 2));
}

#[test]
fn bedrock_with_an_unreadable_steps_option_is_a_typed_failure() {
    let response = billing(1);
    let mut options = OptionalParams::new();
    options.insert("steps".to_owned(), Json::Text("many".to_owned()));
    let mut call = image_call(
        "bedrock",
        "stability.sdxl",
        ImageResponseView::Image(&response),
    );
    call.optional_params = Some(&options);
    assert!(matches!(
        price(&call, &empty()),
        Err(ImageDispatchError::Policy(ImagePolicyError::Bedrock(_)))
    ));
}

#[rstest]
#[case::vertex("vertex_ai", "vertex_ai")]
#[case::recraft("recraft", "recraft")]
#[case::aiml("aiml", "aiml")]
#[case::cometapi("cometapi", "cometapi")]
#[case::gemini("gemini", "gemini")]
#[case::fal("fal_ai", "fal_ai")]
#[case::runwayml("runwayml", "runwayml")]
fn a_provider_without_an_installed_calculator_is_a_typed_unavailable_adapter(
    #[case] provider: &str,
    #[case] name: &str,
) {
    let response = billing(1);
    let call = image_call(provider, "model", ImageResponseView::Image(&response));
    let Err(ImageDispatchError::Policy(ImagePolicyError::UnavailableAdapter(found))) =
        price(&call, &empty())
    else {
        panic!("expected an unavailable adapter for {provider}");
    };
    assert_eq!(found, name);
}

#[test]
fn a_gemini_edit_names_the_same_unavailable_adapter() {
    let response = billing(1);
    let mut call = image_call("gemini", "model", ImageResponseView::Image(&response));
    call.call_type = Some("image_edit");
    assert!(matches!(
        price(&call, &empty()),
        Err(ImageDispatchError::Policy(
            ImagePolicyError::UnavailableAdapter("gemini")
        ))
    ));
}

#[test]
fn the_default_calculator_refuses_a_route_that_is_not_the_default_one() {
    let database = TimeZoneDatabase::from_env();
    let lookup = empty();
    let policy = ProviderImagePricing {
        lookup: &lookup,
        database: &database,
        moment: DateTime::<Utc>::UNIX_EPOCH,
    };
    let response = billing(1);
    let route = ImageRoute::GptImage {
        image: PricedImage {
            model: "gpt-image-1",
            response: ImageResponseView::Image(&response),
            prices: None,
        },
        provider: "openai",
    };
    assert!(matches!(
        policy.default_price(&route),
        Err(ImagePolicyError::UnavailableAdapter("gpt_image"))
    ));
}
