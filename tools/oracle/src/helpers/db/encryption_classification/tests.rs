#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;

#[test]
fn classification_uses_the_supplied_salt() {
    let input: Input = serde_json::from_value(serde_json::json!({
        "salt_key": "alternate-scan-key",
        "values": ["AAECAwQFBgcICQoLDA0ODxAREhMUFRYXvDOJyJMw/xKuXGxKjbDJovT6e8AVaQ=="]
    }))
    .unwrap();
    let result = Op::run(&input).unwrap();
    assert_eq!(result["counts"]["legacy"], 1_u64);
    assert_eq!(result["counts"]["plaintext"], 0_u64);
}
