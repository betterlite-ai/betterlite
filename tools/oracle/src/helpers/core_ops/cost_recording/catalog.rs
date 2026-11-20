use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::off_peak::schema::OffPeakBlockEntry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::{Scalar, Tier};
use serde::de::IgnoredAny;

pub(super) struct Catalog {
    entries: BTreeMap<String, PricedModel>,
}

impl Catalog {
    pub(super) fn new(entries: &BTreeMap<String, BTreeMap<String, Json>>) -> Self {
        let entries = entries
            .iter()
            .map(|(model, fields)| (model.clone(), priced_model(model, fields)))
            .collect();
        Self { entries }
    }
}

// The tables stay decoded, as CatalogLookup keeps them; PriceRecord alone would lose them.
fn priced_model(model: &str, fields: &BTreeMap<String, Json>) -> PricedModel {
    let mut record: PriceRecord = fields
        .iter()
        .map(|(name, value)| (name.clone(), scalar(value)))
        .collect();
    record.insert("key".to_owned(), Scalar::Text(model.to_owned()));
    let tiered: Option<Vec<Tier>> = fields
        .get("tiered_pricing")
        .and_then(|value| serde_json::from_slice(&serde_json::to_vec(value).ok()?).ok());
    let off_peak: Option<OffPeakBlockEntry> = fields
        .get("off_peak_pricing")
        .and_then(|value| serde_json::from_slice(&serde_json::to_vec(value).ok()?).ok());
    PricedModel {
        record,
        tiered_pricing: tiered.unwrap_or_default(),
        off_peak_pricing: off_peak.and_then(|entry| entry.as_block().cloned()),
        off_peak_cache_read_present: matches!(
            fields.get("off_peak_pricing"),
            Some(Json::Object(block)) if block.contains_key("cache_read_input_token_cost")
        ),
    }
}

impl PriceLookup for Catalog {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.entries.get(model).map(|priced| priced.record.clone())
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).map(|priced| priced.record.clone())
    }

    fn effective_priced_model(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.entries.get(model).cloned()
    }
}

fn scalar(value: &Json) -> Scalar {
    match value {
        Json::Null => Scalar::Null,
        Json::Bool(flag) => Scalar::Bool(*flag),
        Json::Number(number) => Scalar::Number(number.clone()),
        Json::Text(text) => Scalar::Text(text.clone()),
        Json::List(_) => Scalar::Array(Vec::new()),
        Json::Object(_) => Scalar::Object(IgnoredAny),
    }
}
