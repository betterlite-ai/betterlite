#![cfg(test)]

use super::{OUTPUT_BYTES, check, finish};
use crate::builtin::{Calls, Content, Message};
use indexmap::IndexMap;
use rstest::rstest;

#[rstest]
#[case::input_exact(65_509, true)]
#[case::input_over(65_510, false)]
fn factory_bounds_behavior_tests_behavior(#[case] size: usize, #[case] accepted: bool) {
    let messages = [Message {
        role: "u".to_owned(),
        content: Content::Text("x".repeat(size)),
        tool_calls: Calls::Absent,
        extra: IndexMap::new(),
    }];
    assert_eq!(check(&messages).is_ok(), accepted);
}

#[rstest]
#[case::output_exact(OUTPUT_BYTES, true)]
#[case::output_over(0x0010_0001, false)]
fn factory_bounds_output_tests_behavior(#[case] size: usize, #[case] accepted: bool) {
    assert_eq!(finish("x".repeat(size)).is_ok(), accepted);
}
