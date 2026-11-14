use std::collections::BTreeMap;

use betterlite_models::guardrail_pricing::{
    AZURE_TEXT_RECORD_UNIT, GuardrailPricingError, azure_prompt_shield_cost,
};
use rust_decimal::Decimal;

use super::cost::CostTier;
use crate::azure_content_safety::constants::TEXT_RECORD_LENGTH;
use crate::guardrail_records::record::GuardrailTracingDetail;

pub const REQUESTS_UNIT: &str = "requests";
pub const INPUT_CHARACTERS_UNIT: &str = "input_characters";

// One invocation's counters. LiteLLM keeps them in a ContextVar; the caller owns this value instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BillingUsage {
    requests: i64,
    input_characters: i64,
    text_records: i64,
}

impl BillingUsage {
    // Azure bills each submitted chunk: one request, its characters, and ceil(characters / 1000) records.
    pub fn record_chunk(&mut self, chunk: &str) {
        let length = chunk.chars().count();
        let records = length.div_ceil(TEXT_RECORD_LENGTH);
        let characters = i64::try_from(length).unwrap_or(i64::MAX);
        let records = i64::try_from(records).unwrap_or(i64::MAX);
        self.requests = self.requests.saturating_add(1);
        self.input_characters = self.input_characters.saturating_add(characters);
        self.text_records = self.text_records.saturating_add(records);
    }

    // None is LiteLLM's None stash: an invocation that scanned nothing records no usage at all.
    pub fn into_units(self) -> Option<BTreeMap<String, i64>> {
        if self.requests == 0 {
            return None;
        }
        let mut units = BTreeMap::new();
        units.insert(REQUESTS_UNIT.to_owned(), self.requests);
        units.insert(INPUT_CHARACTERS_UNIT.to_owned(), self.input_characters);
        units.insert(AZURE_TEXT_RECORD_UNIT.to_owned(), self.text_records);
        Some(units)
    }
}

// LiteLLM's _pop_billing_tracing_detail: a priced usage also carries the cost, never counted in spend.
pub fn billing_tracing_detail(
    usage: BillingUsage,
    cost_tier: Option<CostTier>,
    price_per_1000_text_records: Option<Decimal>,
) -> Result<Option<GuardrailTracingDetail>, GuardrailPricingError> {
    let Some(units) = usage.into_units() else {
        return Ok(None);
    };
    let cost = azure_prompt_shield_cost(
        &units,
        cost_tier.map(CostTier::wire_name),
        price_per_1000_text_records,
    )?;
    Ok(Some(GuardrailTracingDetail {
        guardrail_id: None,
        policy_template: None,
        detection_method: None,
        confidence_score: None,
        classification: None,
        match_details: None,
        patterns_checked: None,
        alert_recipients: None,
        risk_score: None,
        violation_categories: None,
        guardrail_action: None,
        guardrail_usage: Some(units),
        guardrail_cost: cost,
        guardrail_cost_by_unit: None,
        guardrail_cost_in_spend: cost.map(|_amount| false),
    }))
}
