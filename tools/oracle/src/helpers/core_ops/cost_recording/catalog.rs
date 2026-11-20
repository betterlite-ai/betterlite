use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use serde::de::IgnoredAny;

pub(super) struct Catalog {
    entries: BTreeMap<String, PriceRecord>,
}

impl Catalog {
    pub(super) fn new(entries: &BTreeMap<String, BTreeMap<String, Json>>) -> Self {
        let records = entries
            .iter()
            .map(|(model, fields)| {
                let mut record: PriceRecord = fields
                    .iter()
                    .map(|(name, value)| (name.clone(), scalar(value)))
                    .collect();
                record.insert("key".to_owned(), Scalar::Text(model.clone()));
                (model.clone(), record)
            })
            .collect();
        Self { entries: records }
    }
}

impl PriceLookup for Catalog {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.entries.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.effective_model_info(model, custom_llm_provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
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
