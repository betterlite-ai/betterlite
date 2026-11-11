#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::Number;

use crate::controls::cheapest::pick_cheapest_chat_models;
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

struct FakeCatalog {
    provider: &'static str,
    entries: BTreeMap<&'static str, PriceRecord>,
}

impl FakeCatalog {
    fn entry(&self, model: &str, custom_llm_provider: Option<&str>) -> Option<&PriceRecord> {
        if custom_llm_provider != Some(self.provider) {
            return None;
        }
        self.entries.get(model)
    }
}

impl PriceLookup for FakeCatalog {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.entry(model, custom_llm_provider).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        let record = self.entry(model, custom_llm_provider)?.clone();
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

fn price(text: &str) -> Scalar {
    Scalar::Number(serde_json::from_str::<Number>(text).expect("a test price parses"))
}

fn chat_entry(input: Option<&str>, output: Option<&str>) -> PriceRecord {
    let mut record = PriceRecord::new();
    record.insert("mode".to_owned(), Scalar::Text("chat".to_owned()));
    if let Some(input_price) = input {
        record.insert("input_cost_per_token".to_owned(), price(input_price));
    }
    if let Some(output_price) = output {
        record.insert("output_cost_per_token".to_owned(), price(output_price));
    }
    record
}

fn catalog() -> FakeCatalog {
    let mut entries = BTreeMap::new();
    entries.insert("chat-cheap", chat_entry(Some("0.05"), Some("0.05")));
    entries.insert("chat-pricey", chat_entry(Some("1.0"), Some("2.0")));
    entries.insert("chat-tie-first", chat_entry(Some("0.1"), Some("0.1")));
    entries.insert("chat-tie-second", chat_entry(Some("0.1"), Some("0.1")));
    entries.insert("unpriced-chat", chat_entry(None, None));
    let mut embedding = PriceRecord::new();
    embedding.insert("mode".to_owned(), Scalar::Text("embedding".to_owned()));
    embedding.insert("input_cost_per_token".to_owned(), price("0.0"));
    entries.insert("embedding-model", embedding);
    let mut no_mode = PriceRecord::new();
    no_mode.insert("input_cost_per_token".to_owned(), price("0.0"));
    entries.insert("no-mode-model", no_mode);
    FakeCatalog {
        provider: "openai",
        entries,
    }
}

#[rstest]
#[case::picks_the_cheapest_chat_model(
    "openai",
    Some(&["chat-pricey", "chat-cheap"][..]),
    1,
    &["chat-cheap"]
)]
#[case::returns_the_requested_count_in_cost_order(
    "openai",
    Some(&["chat-pricey", "chat-cheap"][..]),
    2,
    &["chat-cheap", "chat-pricey"]
)]
#[case::a_tie_keeps_the_provider_order(
    "openai",
    Some(&["chat-tie-second", "chat-tie-first"][..]),
    2,
    &["chat-tie-second", "chat-tie-first"]
)]
#[case::missing_and_non_chat_models_are_skipped(
    "openai",
    Some(&["missing-model", "embedding-model", "no-mode-model"][..]),
    3,
    &[]
)]
#[case::an_unknown_model_info_is_skipped(
    "openai",
    Some(&["missing-model", "chat-cheap"][..]),
    1,
    &["chat-cheap"]
)]
#[case::a_model_without_prices_ranks_as_free(
    "openai",
    Some(&["unpriced-chat", "chat-cheap"][..]),
    1,
    &["unpriced-chat"]
)]
#[case::zero_asked_models_returns_nothing(
    "openai",
    Some(&["chat-cheap"][..]),
    0,
    &[]
)]
#[case::more_models_than_known_returns_every_chat_model(
    "openai",
    Some(&["chat-pricey", "embedding-model", "unpriced-chat", "chat-cheap"][..]),
    5,
    &["unpriced-chat", "chat-cheap", "chat-pricey"]
)]
#[case::another_provider_finds_no_models(
    "azure",
    Some(&["chat-cheap"][..]),
    1,
    &[]
)]
#[case::an_unknown_provider_has_no_known_models(
    "openai",
    None,
    1,
    &[]
)]
fn provider_controls_tests_behavior(
    #[case] provider: &str,
    #[case] models: Option<&[&str]>,
    #[case] count: usize,
    #[case] expected: &[&str],
) {
    let known = models.map(|listed| {
        listed
            .iter()
            .map(|model| (*model).to_owned())
            .collect::<Vec<String>>()
    });
    let picked = pick_cheapest_chat_models(provider, count, known.as_deref(), &catalog());
    let wanted: Vec<String> = expected.iter().map(|model| (*model).to_owned()).collect();
    assert_eq!(picked, wanted);
}
