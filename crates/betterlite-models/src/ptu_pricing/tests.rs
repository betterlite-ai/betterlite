#![cfg(test)]

mod config_errors;
mod corpus;
mod declares;
mod identity;
mod numeric;
mod spillover;
mod terms;
mod zeroed;

use std::collections::BTreeMap;

use crate::tiered_pricing::Scalar;

fn model_info(source: &str) -> BTreeMap<String, Scalar> {
    serde_json::from_str(source).unwrap()
}

fn declaration(fields: &str) -> String {
    format!(r#"{{"ptu_effective_from":"2026-01-01T00:00:00Z",{fields}}}"#)
}
