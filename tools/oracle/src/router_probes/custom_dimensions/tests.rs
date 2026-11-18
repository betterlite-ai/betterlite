#![cfg(test)]

use super::{Input, ProbeConfig, resolved};

#[test]
fn an_omitted_classifier_type_defaults_to_heuristic_in_probe_config() {
    let input: Input = serde_json::from_str(
        r#"{
        "function": "validate",
        "config": {
            "custom_dimensions": [{
                "name": "domain",
                "weight": 0.5,
                "keywords": ["topic"]
            }]
        }
    }"#,
    )
    .expect("the probe config is valid without classifier_type");
    let serialized = serde_json::to_value(input).expect("the probe config serializes");
    assert_eq!(serialized["config"]["classifier_type"], "heuristic");
    let config: ProbeConfig = serde_json::from_str(
        r#"{
        "custom_dimensions": [{
            "name": "domain",
            "weight": 0.5,
            "keywords": ["topic"]
        }]
    }"#,
    )
    .expect("the probe config defaults its classifier type");
    let (scoring, work) = resolved(Some(&config)).unwrap();
    assert_eq!(scoring.custom_dimensions.len(), 1);
    assert_eq!(work, 0);
}
