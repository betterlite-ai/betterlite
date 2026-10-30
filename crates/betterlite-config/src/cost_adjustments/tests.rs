#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

#[test]
fn loads_cost_adjustments() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "litellm_settings:\n  cost_discount_config:\n    vertex_ai: 0.2\n  cost_margin_config:\n    vertex_ai: null\n    global: {percentage: '0.1', fixed_amount: 0.01, ignored: 4}\n",
    )
    .unwrap();
    let settings = &config.sections.cost_adjustments;
    assert_eq!(
        settings.cost_discount_config["vertex_ai"],
        serde_json::json!(0.2)
    );
    assert!(settings.cost_margin_config["vertex_ai"].is_null());
    assert_eq!(settings.cost_margin_config["global"]["percentage"], "0.1");
    let defaults = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .unwrap();
    assert!(
        defaults
            .sections
            .cost_adjustments
            .cost_discount_config
            .is_empty()
    );
    assert!(
        defaults
            .sections
            .cost_adjustments
            .cost_margin_config
            .is_empty()
    );
}
