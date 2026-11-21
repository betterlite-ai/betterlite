use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageHttpResponse};
use serde_json::{Number, from_str};

use crate::stability_edits::error::StabilityEditError;
use crate::stability_edits::reply::ModelPriceLookup;
use crate::stability_edits::types::{EditOptions, EditValue};

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const INPAINT: &str = "stability.stable-image-inpaint-v1:0";
pub(super) const STYLE_TRANSFER: &str = "stability.stable-style-transfer-v1:0";
pub(super) const UNKNOWN: &str = "no.such-model-v0:0";

pub(super) fn json(source: &str) -> Json {
    from_str(source).expect("the fixture is JSON")
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn integer(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn options(entries: &[(&str, Json)]) -> EditOptions {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), EditValue::Json(value.clone())))
        .collect()
}

pub(super) fn with_bytes(mut options: EditOptions, name: &str, content: &[u8]) -> EditOptions {
    options.insert(name.to_owned(), EditValue::Bytes(content.to_vec()));
    options
}

pub(super) fn upstream(status: u16, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: status,
        headers: ImageHeaders::from([("x-a".to_owned(), "b".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn provider_message(error: &StabilityEditError) -> &str {
    match error {
        StabilityEditError::Provider { error, .. } => &error.error.message,
        other => panic!("expected a provider error, got {other}"),
    }
}

pub(super) fn price_record(entries: &[(&str, Scalar)]) -> PriceRecord {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn number(lexeme: &str) -> Scalar {
    Scalar::Number(from_str(lexeme).expect("the fixture is a number"))
}

pub(super) struct Prices(BTreeMap<String, PriceRecord>);

impl Prices {
    pub(super) fn new(entries: &[(&str, PriceRecord)]) -> Self {
        Self(
            entries
                .iter()
                .map(|(name, record)| ((*name).to_owned(), record.clone()))
                .collect(),
        )
    }

    pub(super) fn catalog() -> Self {
        Self::new(&[
            (
                INPAINT,
                price_record(&[("output_cost_per_image", number("0.07"))]),
            ),
            (
                STYLE_TRANSFER,
                price_record(&[("output_cost_per_image", number("0.08"))]),
            ),
        ])
    }
}

impl ModelPriceLookup for Prices {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        self.0.get(model).cloned()
    }
}
