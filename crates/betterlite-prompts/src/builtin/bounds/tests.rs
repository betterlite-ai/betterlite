#![cfg(test)]

use super::{OUTPUT_BYTES, check, finish};
use crate::builtin::{Content, Message, PromptError, deepseek};
use rstest::rstest;

#[derive(Clone, Copy)]
enum Boundary {
    Input,
    Output,
    Dependency,
}

#[rstest]
#[case::input_exact(Boundary::Input, 65_509, true)]
#[case::input_over(Boundary::Input, 65_510, false)]
#[case::output_exact(Boundary::Output, OUTPUT_BYTES, true)]
#[case::output_over(Boundary::Output, 0x0010_0001, false)]
#[case::injected_model_and_messages(Boundary::Dependency, 5, true)]
#[case::injected_failure(Boundary::Dependency, 5, false)]
fn builtin_prompts_behavior_tests_behavior(
    #[case] boundary: Boundary,
    #[case] size: usize,
    #[case] accepted: bool,
) {
    let messages = [Message {
        role: "u".to_owned(),
        content: Content::Text("x".repeat(size)),
        tool_calls: Vec::new(),
    }];
    match boundary {
        Boundary::Input => assert_eq!(check(&messages).is_ok(), accepted),
        Boundary::Output => {
            assert_eq!(finish("x".repeat(size)).is_ok(), accepted);
            assert_eq!(deepseek(&[], |_, _| Ok("x".repeat(size))).is_ok(), accepted);
        }
        Boundary::Dependency => {
            let actual = deepseek(&messages, |model, turns| {
                assert_eq!(model, "deepseek-r1/deepseek-r1-7b-instruct");
                assert_eq!(turns, messages);
                if accepted {
                    Ok("rendered".to_owned())
                } else {
                    Err(PromptError::Empty)
                }
            });
            if accepted {
                assert_eq!(actual.unwrap(), "rendered");
            } else {
                assert!(matches!(actual, Err(PromptError::Empty)));
            }
        }
    }
}
