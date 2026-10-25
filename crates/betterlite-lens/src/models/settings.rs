use garde::Validate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;

use crate::models::check_validation::check_rules;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    pub team_id: String,
    pub api_key_hash: String,
    pub all_teams: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct MetadataFilter {
    #[garde(length(chars, min = 1, max = 200))]
    pub key: String,
    #[garde(length(chars, min = 1, max = 500))]
    pub value: String,
}

#[serde_inline_default]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct Check {
    #[garde(length(chars, min = 1, max = 80))]
    pub id: String,
    #[garde(length(chars, min = 3, max = 3000))]
    pub instruction: String,
    #[serde_inline_default(true)]
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Traces,
    Requests,
    Both,
}

pub fn positive(amount: &Decimal, (): &()) -> garde::Result {
    if *amount > Decimal::ZERO {
        Ok(())
    } else {
        Err(garde::Error::new("must be greater than 0"))
    }
}

#[serde_inline_default]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
#[garde(allow_unvalidated)]
pub struct LensSettings {
    #[garde(length(chars, min = 1, max = 100))]
    pub name: String,
    #[serde(default)]
    #[garde(length(chars, max = 6000))]
    pub context: String,
    #[serde(default)]
    pub source: Source,
    #[serde_inline_default(24)]
    #[garde(range(min = 1, max = 8760))]
    pub lookback_hours: u32,
    #[serde(default)]
    #[garde(length(chars, max = 200))]
    pub service: String,
    #[serde(default)]
    #[garde(length(chars, max = 200))]
    pub agent_name: String,
    #[serde(default)]
    #[garde(length(max = 8), dive)]
    pub filters: Vec<MetadataFilter>,
    #[serde(default)]
    #[garde(dive, custom(check_rules(&self.context)))]
    pub checks: Vec<Check>,
    #[garde(length(chars, min = 1, max = 200))]
    pub model: String,
    #[serde_inline_default(true)]
    pub enabled: bool,
    #[serde_inline_default(15)]
    #[garde(range(min = 1, max = 10080))]
    pub interval_minutes: u32,
    // absent means every eligible execution, which no count can say
    #[serde(default)]
    #[garde(range(min = 1))]
    pub sample_size: Option<u64>,
    #[serde_inline_default(Decimal::ONE_HUNDRED)]
    #[serde(with = "rust_decimal::serde::float")]
    #[garde(custom(positive), range(max = Decimal::ONE_HUNDRED))]
    pub sample_percent: Decimal,
    #[serde_inline_default(8)]
    #[garde(range(min = 1))]
    pub concurrency: u64,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub execution_ids: Vec<String>,
    #[serde_inline_default(Decimal::ONE_HUNDRED)]
    #[serde(with = "rust_decimal::serde::float")]
    #[garde(custom(positive), range(max = Decimal::from(100_000_u32)))]
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
