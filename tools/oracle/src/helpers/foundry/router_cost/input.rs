use std::collections::BTreeMap;

use betterlite_models::price_lookup::PriceRecord;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Classify {
        calls: Vec<ClassifyCall>,
    },
    Flat {
        #[serde(default)]
        catalog: BTreeMap<String, PriceRecord>,
        calls: Vec<FlatCall>,
    },
    Cost {
        #[serde(default)]
        catalog: BTreeMap<String, PriceRecord>,
        calls: Vec<CostCall>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassifyCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) request_model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlatCall {
    pub(crate) model: String,
    pub(crate) prompt_tokens: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CostCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) request_model: Option<String>,
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    #[serde(default)]
    pub(crate) service_tier: Option<String>,
    #[serde(default)]
    pub(crate) response_time_ms: Option<Decimal>,
}
