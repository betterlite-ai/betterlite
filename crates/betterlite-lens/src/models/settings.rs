use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;

use crate::models::check_validation::check_rules;
use crate::models::validate::{RecordError, Validate, check_items, check_length};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    pub team_id: String,
    pub api_key_hash: String,
    pub all_teams: bool,
}

impl Validate for Scope {}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataFilter {
    pub key: String,
    pub value: String,
}

impl Validate for MetadataFilter {
    fn validate(&self) -> Result<(), RecordError> {
        check_length("key", &self.key, 1, 200)?;
        check_length("value", &self.value, 1, 500)
    }
}

#[serde_inline_default]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub instruction: String,
    #[serde_inline_default(true)]
    pub enabled: bool,
}

impl Validate for Check {
    fn validate(&self) -> Result<(), RecordError> {
        check_length("id", &self.id, 1, 80)?;
        check_length("instruction", &self.instruction, 3, 3000)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Traces,
    Requests,
    Both,
}

#[serde_inline_default]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LensSettings {
    pub name: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub source: Source,
    #[serde_inline_default(24)]
    pub lookback_hours: u32,
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub filters: Vec<MetadataFilter>,
    #[serde(default)]
    pub checks: Vec<Check>,
    pub model: String,
    #[serde_inline_default(true)]
    pub enabled: bool,
    #[serde_inline_default(15)]
    pub interval_minutes: u32,
    // absent means every eligible execution, which no count can say
    #[serde(default)]
    pub sample_size: Option<u64>,
    #[serde_inline_default(Decimal::ONE_HUNDRED)]
    #[serde(with = "rust_decimal::serde::float")]
    pub sample_percent: Decimal,
    #[serde_inline_default(8)]
    pub concurrency: u64,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub execution_ids: Vec<String>,
    #[serde_inline_default(Decimal::ONE_HUNDRED)]
    #[serde(with = "rust_decimal::serde::float")]
    pub monthly_budget: Decimal,
}

impl LensSettings {
    #[must_use]
    pub fn analysis_checks(&self) -> Vec<Check> {
        let behavior = Check {
            id: "expected_behavior".to_owned(),
            instruction: "Identify deviations from the expected behavior described in context."
                .to_owned(),
            enabled: true,
        };
        let described = (!self.context.trim().is_empty()).then_some(behavior);
        let enabled = self.checks.iter().filter(|check| check.enabled).cloned();
        described.into_iter().chain(enabled).collect()
    }
}

impl Validate for LensSettings {
    fn validate(&self) -> Result<(), RecordError> {
        check_length("name", &self.name, 1, 100)?;
        check_length("context", &self.context, 0, 6000)?;
        check_length("service", &self.service, 0, 200)?;
        check_length("agent_name", &self.agent_name, 0, 200)?;
        check_length("model", &self.model, 1, 200)?;
        check_items("filters", &self.filters, 0, 8)?;
        self.checks.iter().try_for_each(Validate::validate)?;
        let ranges = [
            ("lookback_hours", (1..=8760).contains(&self.lookback_hours)),
            (
                "interval_minutes",
                (1..=10080).contains(&self.interval_minutes),
            ),
            ("sample_size", self.sample_size != Some(0)),
            ("concurrency", self.concurrency >= 1),
            (
                "sample_percent",
                self.sample_percent > Decimal::ZERO && self.sample_percent <= Decimal::ONE_HUNDRED,
            ),
            (
                "monthly_budget",
                self.monthly_budget > Decimal::ZERO
                    && self.monthly_budget <= Decimal::from(100_000_u32),
            ),
        ];
        if let Some((field, _)) = ranges.into_iter().find(|(_, within)| !within) {
            return Err(RecordError::Range { field });
        }
        check_rules(self)
    }
}
