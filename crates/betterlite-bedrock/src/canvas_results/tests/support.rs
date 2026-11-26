use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse,
};
use serde_json::from_str;

use crate::bedrock_settings::BedrockError;
use crate::canvas_edits::error::CanvasEditError;
use crate::canvas_results::{
    CanvasEditReply, CanvasResultError, ModelPriceLookup, transform_image_edit_response,
};

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const MODEL: &str = "amazon.nova-canvas-v1:0";
pub(super) const OTHER_MODEL: &str = "amazon.nova-canvas-v1:1";

pub(super) fn json(source: &str) -> Json {
    from_str(source).expect("the fixture is JSON")
}

pub(super) fn record(source: &str) -> PriceRecord {
    from_str(source).expect("the fixture is a price record")
}

pub(super) fn upstream(status: u16, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: status,
        headers: ImageHeaders::from([("x-a".to_owned(), "b".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn unpriced(status: u16, body: &str) -> Result<CanvasEditReply, CanvasResultError> {
    transform_image_edit_response(MODEL, &upstream(status, body), NOW, &Prices::new(&[]))
}

pub(super) fn priced(
    model: &str,
    price: PriceRecord,
    status: u16,
    body: &str,
) -> Result<CanvasEditReply, CanvasResultError> {
    transform_image_edit_response(
        model,
        &upstream(status, body),
        NOW,
        &Prices::new(&[(model, price)]),
    )
}

pub(super) fn image(value: &str) -> ImageData {
    ImageData {
        b64_json: Some(value.to_owned()),
        revised_prompt: None,
        url: None,
    }
}

pub(super) fn provider_error(error: &CanvasResultError) -> &BedrockError {
    match error {
        CanvasResultError::Edit(CanvasEditError::Provider { error: carried, .. }) => carried,
        other @ (CanvasResultError::Edit(
            CanvasEditError::MissingImage
            | CanvasEditError::TupleFileType
            | CanvasEditError::OutPaintingMask
            | CanvasEditError::InPaintingMask
            | CanvasEditError::UnsupportedTaskType { .. }
            | CanvasEditError::MissingPrompt { .. }
            | CanvasEditError::Field { .. }
            | CanvasEditError::UnsupportedModel { .. }
            | CanvasEditError::CompleteUrl
            | CanvasEditError::Header(_),
        )
        | CanvasResultError::Reply(_)) => panic!("expected a provider error, got {other}"),
    }
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
}

impl ModelPriceLookup for Prices {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        self.0.get(model).cloned()
    }
}
