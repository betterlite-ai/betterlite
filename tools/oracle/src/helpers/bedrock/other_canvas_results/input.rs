use std::collections::BTreeMap;

use betterlite_models::price_lookup::PriceRecord;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) model_cost: BTreeMap<String, PriceRecord>,
    pub(crate) calls: Vec<ResponseCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseCall {
    pub(crate) model: String,
    pub(crate) status: u16,
    pub(crate) body: String,
}
