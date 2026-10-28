#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;

#[rstest]
#[case::default(
    r#"{"family":"default","messages":[{"role":"user","content":"u"}]}"#,
    "u"
)]
#[case::deepseek(
    r#"{"family":"deepseek","messages":[{"role":"user","content":"u"}]}"#,
    "deepseek-r1/deepseek-r1-7b-instruct:u"
)]
fn builtin_prompts_behavior_tests_behavior(#[case] wire: &str, #[case] expected: &str) {
    let input: Input = serde_json::from_str(wire).unwrap();
    assert_eq!(
        Op::run(&input).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn errors_keep_the_python_class() {
    let input: Input = serde_json::from_str(
        r#"{"family":"ollama","messages":[{"role":"other","content":"x"}],"model":"chosen"}"#,
    )
    .unwrap();
    let error = Op::run(&input).unwrap_err();
    assert_eq!(error.class, "BadRequestError");
    assert_eq!(error.status, None);
    assert!(error.message.contains("chosen"));
}
