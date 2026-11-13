use rust_decimal::Decimal;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::str::FromStr as _;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PrometheusSample {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub timestamp: Decimal,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PrometheusRangeSample {
    pub metric: BTreeMap<String, String>,
    pub values: Vec<PrometheusSample>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PrometheusQueryData {
    pub result: Vec<PrometheusRangeSample>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PrometheusQueryResponse {
    pub data: PrometheusQueryData,
}

impl PrometheusSample {
    #[must_use]
    pub fn numeric_value(&self) -> Option<Decimal> {
        if let Ok(decimal) = Decimal::from_str(&self.value) {
            return Some(decimal);
        }
        Decimal::from_scientific(&self.value).ok()
    }
}

mod tests;
