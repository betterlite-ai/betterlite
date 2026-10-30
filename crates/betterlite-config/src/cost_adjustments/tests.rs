#![cfg(test)]

use std::collections::BTreeMap;

use crate::cost_adjustments::CostAdjustmentValue;
use crate::loader::ConfigLoader;

#[test]
fn loads_cost_adjustments() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "litellm_settings:\n  cost_discount_config:\n    vertex_ai: 0.2\n  cost_margin_config:\n    vertex_ai: null\n    global: {percentage: '0.1', fixed_amount: 0.01}\n",
    )
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_discount_config["vertex_ai"],
        CostAdjustmentValue::Number(serde_json::Number::from_f64(0.2).unwrap())
    );
    assert_eq!(
        settings.cost_margin_config["vertex_ai"],
        CostAdjustmentValue::Null
    );
    assert_eq!(
        settings.cost_margin_config["global"],
        CostAdjustmentValue::Object(BTreeMap::from([
            (
                "percentage".to_owned(),
                CostAdjustmentValue::Text("0.1".to_owned()),
            ),
            (
                "fixed_amount".to_owned(),
                CostAdjustmentValue::Number(serde_json::Number::from_f64(0.01).unwrap()),
            ),
        ]))
    );
    let defaults = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .unwrap();
    let default_settings = &defaults.sections.cost_adjustments;
    assert!(default_settings.cost_discount_config.is_empty());
    assert!(default_settings.cost_margin_config.is_empty());
}
