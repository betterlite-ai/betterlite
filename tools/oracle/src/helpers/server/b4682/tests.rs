#![cfg(test)]

use super::{Input, Op};

#[test]
fn override_input_uses_the_production_service() {
    let input = serde_json::json!({
        "operation": "override",
        "prefix": "srv",
        "tools": [{"name": "srv-search", "description": "old", "input_schema": {}}],
        "overrides": {"search": "safe"}
    });
    let decoded: Input = serde_json::from_value(input).expect("typed override input");
    let result = <Op as crate::helpers::Helper>::run(&decoded).expect("override succeeds");
    assert_eq!(result[0]["description"], "safe");
}
