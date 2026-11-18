#![cfg(test)]

use serde_json::json;

use super::{Input, ProbeConfig, resolved};

#[test]
fn an_omitted_classifier_type_defaults_to_heuristic_in_probe_config() {
    let input: Input = serde_json::from_value(json!({
        "function": "validate",
        "config": {
            "custom_dimensions": [{
                "name": "domain",
                "weight": 0.5_f64,
                "keywords": ["topic"]
            }]
        }
    }))
    .expect("the probe config is valid without classifier_type");
    let serialized = serde_json::to_value(input).expect("the probe config serializes");
    assert_eq!(serialized["config"]["classifier_type"], "heuristic");
    let config: ProbeConfig = serde_json::from_value(json!({
        "custom_dimensions": [{
            "name": "domain",
            "weight": 0.5_f64,
            "keywords": ["topic"]
        }]
    }))
    .expect("the probe config defaults its classifier type");
    assert!(resolved(Some(&config)).is_ok());
}
