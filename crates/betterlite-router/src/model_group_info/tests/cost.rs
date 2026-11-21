use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use serde_json::Number;

use super::decimal;
use crate::model_group_info::cost::cost_value;

#[test]
fn cost_scalar_conversion() {
    assert_eq!(cost_value(None).expect("cost"), None);
    assert_eq!(cost_value(Some(&Json::Null)).expect("cost"), None);
    assert_eq!(
        cost_value(Some(&Json::Bool(true))).expect("cost"),
        Some(Decimal::ONE)
    );
    assert_eq!(
        cost_value(Some(&Json::Bool(false))).expect("cost"),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        cost_value(Some(&Json::Number(Number::from(0_i64)))).expect("cost"),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        cost_value(Some(&Json::Text(" 2_000.5 ".to_owned()))).expect("cost"),
        Some(decimal("2000.5"))
    );
    assert_eq!(
        cost_value(Some(&Json::Text("1e-3".to_owned()))).expect("cost"),
        Some(decimal("0.001"))
    );
    assert_eq!(
        cost_value(Some(&Json::Text("abc".to_owned()))).expect("cost"),
        None
    );
    assert_eq!(
        cost_value(Some(&Json::Text("0x1".to_owned()))).expect("cost"),
        None
    );
    assert_eq!(
        cost_value(Some(&Json::List(Vec::new()))).expect("cost"),
        None
    );
    assert_eq!(
        cost_value(Some(&Json::Object(JsonObject::new()))).expect("cost"),
        None
    );
    cost_value(Some(&Json::Text("inf".to_owned()))).unwrap_err();
    cost_value(Some(&Json::Text("nan".to_owned()))).unwrap_err();
}
