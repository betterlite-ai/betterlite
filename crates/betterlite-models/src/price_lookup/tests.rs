#![cfg(test)]

use super::{PriceLookup, PriceRecord};
use crate::tiered_pricing::Scalar;
use std::collections::BTreeMap;

// An effective-only or raw-only fixture must never answer the other method from the same key.
struct Fixture {
    effective: BTreeMap<String, PriceRecord>,
    raw: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for Fixture {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.effective.get(model).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.raw.get(key).cloned()
    }
}

fn numeric_record(field: &str, value: i64) -> PriceRecord {
    BTreeMap::from([(
        field.to_owned(),
        Scalar::Number(serde_json::Number::from(value)),
    )])
}

#[test]
fn effective_lookup_swallows_an_unresolved_model_as_absent() {
    let fixture = Fixture {
        effective: BTreeMap::new(),
        raw: BTreeMap::new(),
    };
    assert!(
        fixture
            .effective_model_info("unknown/model", None)
            .is_none()
    );
}

#[test]
fn raw_access_never_answers_a_model_only_effective_lookup_resolves() {
    let mut fixture = Fixture {
        effective: BTreeMap::new(),
        raw: BTreeMap::new(),
    };
    fixture
        .effective
        .insert("gpt".to_owned(), numeric_record("ocr_cost_per_page", 1));

    let effective = fixture
        .effective_model_info("gpt", None)
        .expect("fixture provides gpt");
    assert!(matches!(
        effective.get("ocr_cost_per_page"),
        Some(Scalar::Number(_))
    ));
    assert!(fixture.raw_cost_map_entry("gpt").is_none());
}

#[test]
fn raw_access_reads_only_its_own_cost_map_key() {
    let mut fixture = Fixture {
        effective: BTreeMap::new(),
        raw: BTreeMap::new(),
    };
    fixture
        .raw
        .insert("gpt-4".to_owned(), numeric_record("ocr_cost_per_credit", 2));

    let raw = fixture
        .raw_cost_map_entry("gpt-4")
        .expect("fixture provides the raw entry");
    assert!(matches!(
        raw.get("ocr_cost_per_credit"),
        Some(Scalar::Number(_))
    ));
    assert!(fixture.effective_model_info("gpt-4", None).is_none());
}
