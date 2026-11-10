use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::tiered_pricing::Scalar;
use rust_decimal::Decimal;
use serde_json::{Value, json};

pub(super) fn optional_decimal(value: Option<Decimal>) -> Value {
    let Some(amount) = value else {
        return Value::Null;
    };
    Value::from(amount.normalize().to_string())
}

pub(super) fn optional_text(value: Option<&str>) -> Value {
    let Some(text) = value else {
        return Value::Null;
    };
    Value::from(text)
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

pub(super) fn scalar_value(scalar: &Scalar) -> Value {
    match scalar {
        Scalar::Null => Value::Null,
        Scalar::Bool(flag) => Value::Bool(*flag),
        Scalar::Number(number) => match Decimal::from_str(&number.to_string()) {
            Ok(value) => Value::from(value.normalize().to_string()),
            Err(_error) => Value::from(number.to_string()),
        },
        Scalar::Text(text) => Value::from(text.clone()),
        Scalar::Array(_) => Value::from("array"),
        Scalar::Object(_) => Value::from("object"),
    }
}
