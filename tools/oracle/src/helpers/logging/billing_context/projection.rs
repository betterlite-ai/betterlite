use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde_json::{Value, json};

pub(super) fn optional_decimal(value: Option<Decimal>) -> Value {
    let Some(amount) = value else {
        return Value::Null;
    };
    Value::from(amount.normalize().to_string())
}

pub(super) fn optional_costs(costs: Option<&BTreeMap<String, Decimal>>) -> Value {
    let Some(amounts) = costs else {
        return Value::Null;
    };
    let rendered: BTreeMap<String, String> = amounts
        .iter()
        .map(|(key, amount)| (key.clone(), amount.normalize().to_string()))
        .collect();
    json!(rendered)
}
