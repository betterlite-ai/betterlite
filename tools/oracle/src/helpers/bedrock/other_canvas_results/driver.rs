use std::collections::BTreeMap;

use betterlite_bedrock::canvas_results::{
    COST_HEADER, CanvasEditReply, ModelPriceLookup, transform_image_edit_response,
};
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageHttpResponse};
use serde_json::{Value, json};

use super::input::ResponseCall;

// A supplied clock keeps the ImageResponse timestamp out of the comparison.
const NOW_SECONDS: i64 = 1_700_000_000;

pub(super) fn observe(call: &ResponseCall, model_cost: &BTreeMap<String, PriceRecord>) -> Value {
    let upstream = ImageHttpResponse {
        status_code: call.status,
        headers: ImageHeaders::new(),
        body: call.body.as_bytes().to_vec(),
    };
    let prices = Prices(model_cost.clone());
    match transform_image_edit_response(&call.model, &upstream, NOW_SECONDS, &prices) {
        Ok(reply) => json!({
            "ok": true,
            "images": image_entries(&reply),
            "cost": reply.additional_headers.get(COST_HEADER),
        }),
        Err(error) => json!({"ok": false, "status": error.status()}),
    }
}

fn image_entries(reply: &CanvasEditReply) -> Value {
    let mut entries = Vec::new();
    for image in reply.response.data.iter().flatten() {
        if let Some(b64_json) = &image.b64_json {
            entries.push(json!({"b64_json": b64_json}));
        }
    }
    Value::Array(entries)
}

struct Prices(BTreeMap<String, PriceRecord>);

impl ModelPriceLookup for Prices {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        self.0.get(model).cloned()
    }
}
