#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;

#[test]
fn split_leading_system_run_separates_the_leading_run_from_the_rest() {
    let input: Input = serde_json::from_str(
        r#"{"operation":"split_leading_system_run","messages":[{"role":"system","content":"a"},{"role":"user","content":"b"}]}"#,
    )
    .unwrap();
    let value = Op::run(&input).unwrap();
    assert_eq!(
        value,
        serde_json::json!([
            [{"role": "system", "content": "a"}],
            [{"role": "user", "content": "b"}],
        ])
    );
}
