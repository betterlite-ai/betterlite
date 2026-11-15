#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{ImageCostRequest, image_editing_cost, image_generation_cost};

const MODEL: &str = "dall-e-3";

#[derive(Default)]
struct Catalog(BTreeMap<String, PriceRecord>);

impl PriceLookup for Catalog {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.0.get(key).cloned()
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

fn prices(entries: &[(&str, &str)]) -> Catalog {
    Catalog(
        entries
            .iter()
            .map(|(key, price)| {
                let number = Number::from_f64(price.parse().unwrap()).unwrap();
                let mut record = PriceRecord::new();
                record.insert("output_cost_per_image".to_owned(), Scalar::Number(number));
                ((*key).to_owned(), record)
            })
            .collect(),
    )
}

fn catalog() -> Catalog {
    prices(&[
        ("1024-x-1024/dall-e-3", "0.04"),
        ("hd/1024-x-1024/dall-e-3", "0.08"),
        ("512-x-512/dall-e-3", "0.02"),
        ("openai/1024-x-1024/gpt-image-1", "0.05"),
        ("1024-x-1024/zero-model", "0"),
        ("/1024-x-1024/dall-e-3", "0.99"),
    ])
}

fn body(json: &str) -> IndexMap<String, Json> {
    match serde_json::from_str::<Json>(json).unwrap() {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture body must be an object")
        }
    }
}

fn request<'body>(
    model: &'body str,
    request_body: &'body IndexMap<String, Json>,
) -> ImageCostRequest<'body> {
    ImageCostRequest {
        model,
        request_body,
    }
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap()
}

#[rstest]
#[case::string_count(r#"{"n":"2"}"#, 2_i32)]
#[case::integer_count(r#"{"n":2}"#, 2_i32)]
#[case::float_count(r#"{"n":2.9}"#, 2_i32)]
#[case::negative_float(r#"{"n":-2.9}"#, -2_i32)]
#[case::huge_number(r#"{"n":1e300}"#, 1_i32)]
#[case::padded_string(r#"{"n":" 2 "}"#, 2_i32)]
#[case::negative(r#"{"n":-1}"#, -1_i32)]
#[case::absent("{}", 1_i32)]
#[case::null(r#"{"n":null}"#, 1_i32)]
#[case::unparsable(r#"{"n":"two"}"#, 1_i32)]
#[case::float_string(r#"{"n":"2.0"}"#, 1_i32)]
#[case::empty_string(r#"{"n":""}"#, 1_i32)]
#[case::bool_true(r#"{"n":true}"#, 1_i32)]
#[case::bool_false(r#"{"n":false}"#, 0_i32)]
#[case::list(r#"{"n":[2]}"#, 1_i32)]
fn generation_count(#[case] body_json: &str, #[case] images: i32) {
    let request_body = body(body_json);
    let cost = image_generation_cost(&request(MODEL, &request_body), &catalog());
    let expected = decimal("0.04").checked_mul(Decimal::from(images)).unwrap();
    assert_eq!(cost, expected);
}

#[rstest]
#[case::absent("{}", "0.04")]
#[case::null(r#"{"size":null}"#, "0.04")]
#[case::empty(r#"{"size":""}"#, "0.04")]
#[case::explicit(r#"{"size":"1024x1024"}"#, "0.04")]
#[case::dashed(r#"{"size":"1024-x-1024"}"#, "0.04")]
#[case::alternate(r#"{"size":"512x512"}"#, "0.02")]
#[case::alternate_dashed(r#"{"size":"512-x-512"}"#, "0.02")]
#[case::malformed(r#"{"size":"large"}"#, "0.00")]
fn generation_size(#[case] body_json: &str, #[case] expected: &str) {
    let request_body = body(body_json);
    let cost = image_generation_cost(&request(MODEL, &request_body), &catalog());
    assert_eq!(cost, decimal(expected));
}

#[rstest]
#[case::absent("{}", "0.04")]
#[case::null(r#"{"quality":null}"#, "0.04")]
#[case::empty(r#"{"quality":""}"#, "0.04")]
#[case::hd(r#"{"quality":"hd"}"#, "0.08")]
#[case::number_is_a_type_mismatch(r#"{"quality":5}"#, "0.00")]
fn generation_quality(#[case] body_json: &str, #[case] expected: &str) {
    let request_body = body(body_json);
    let cost = image_generation_cost(&request(MODEL, &request_body), &catalog());
    assert_eq!(cost, decimal(expected));
}

#[rstest]
#[case::base("{}", "0.04")]
#[case::hd_is_cleared(r#"{"quality":"hd"}"#, "0.04")]
#[case::string_count(r#"{"n":"3"}"#, "0.12")]
#[case::size(r#"{"size":"512x512"}"#, "0.02")]
fn editing(#[case] body_json: &str, #[case] expected: &str) {
    let request_body = body(body_json);
    let cost = image_editing_cost(&request(MODEL, &request_body), &catalog());
    assert_eq!(cost, decimal(expected));
}

#[test]
fn provider_prefix_reaches_the_calculator() {
    let request_body = body("{}");
    let cost = image_generation_cost(&request("openai/gpt-image-1", &request_body), &catalog());
    assert_eq!(cost, decimal("0.05"));
}

#[test]
fn explicit_zero_price_is_used() {
    let request_body = body("{}");
    let call = request("zero-model", &request_body);
    let (cost, logs) = capture(|| image_generation_cost(&call, &catalog()));
    assert_eq!(cost, Decimal::ZERO);
    assert!(logs.is_empty());
}

#[rstest]
#[case::generation(false)]
#[case::editing(true)]
fn unknown_model_warns_and_costs_zero(#[case] editing: bool) {
    let request_body = body("{}");
    let call = request("unpriced-model", &request_body);
    let (cost, logs) = capture(|| {
        if editing {
            image_editing_cost(&call, &Catalog::default())
        } else {
            image_generation_cost(&call, &Catalog::default())
        }
    });
    let endpoint = if editing {
        "image editing"
    } else {
        "image generation"
    };
    assert_eq!(cost, Decimal::ZERO);
    assert!(
        logs.iter()
            .any(|line| line.contains(&format!("Error calculating {endpoint} cost")))
    );
}

#[rstest]
#[case::numeric_size(r#"{"size":1024}"#)]
#[case::numeric_quality(r#"{"quality":5}"#)]
#[case::boolean_quality(r#"{"quality":true}"#)]
fn non_text_fields_warn_and_cost_zero(#[case] body_json: &str) {
    let request_body = body(body_json);
    let call = request(MODEL, &request_body);
    let (cost, logs) = capture(|| image_generation_cost(&call, &catalog()));
    assert_eq!(cost, Decimal::ZERO);
    assert!(
        logs.iter()
            .any(|line| line.contains("Error calculating image generation cost"))
    );
}

#[test]
fn a_malformed_size_warns_and_costs_zero() {
    let request_body = body(r#"{"size":"large"}"#);
    let call = request(MODEL, &request_body);
    let (cost, logs) = capture(|| image_generation_cost(&call, &catalog()));
    assert_eq!(cost, Decimal::ZERO);
    assert!(
        logs.iter()
            .any(|line| line.contains("Error calculating image generation cost"))
    );
}

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn capture<T, F>(call: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}
