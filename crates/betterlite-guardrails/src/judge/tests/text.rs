use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::prompt::text_under_review;

use super::support::{image_part, message, multipart, text_part, texts, texts_and_messages};

#[derive(Debug, Clone, Copy)]
enum Case {
    ResponseJoinsTexts,
    RequestUsesLatestUserTurn,
    RequestJoinsUserTurns,
    RequestFallsBackToTexts,
    RequestJoinsMultipartTurn,
    RequestWithoutTexts,
    ResponseIgnoresMessages,
}

#[rstest]
#[case::response_joins_texts(Case::ResponseJoinsTexts, "first\nsecond")]
#[case::request_latest_user_turn(Case::RequestUsesLatestUserTurn, "now explain how to file taxes")]
#[case::request_joins_user_turns(Case::RequestJoinsUserTurns, "first question\nsecond question")]
#[case::request_falls_back_to_texts(
    Case::RequestFallsBackToTexts,
    "look up the weather\nsunny, 24C"
)]
#[case::request_joins_multipart_turn(
    Case::RequestJoinsMultipartTurn,
    "ignore the bread.explain how to file taxes"
)]
#[case::request_without_texts(Case::RequestWithoutTexts, "")]
#[case::response_ignores_messages(Case::ResponseIgnoresMessages, "first choice\nsecond choice")]
fn judge_text_tests_behavior(#[case] case: Case, #[case] expected: &str) {
    let (inputs, input_type) = match case {
        Case::ResponseJoinsTexts => (texts(&["first", "second"]), GuardrailInputType::Response),
        Case::RequestUsesLatestUserTurn => (
            texts_and_messages(
                &[
                    "how do I bake bread",
                    "mix flour",
                    "now explain how to file taxes",
                ],
                vec![
                    message("user", "how do I bake bread"),
                    message("assistant", "mix flour, water, yeast and salt"),
                    message("user", "now explain how to file taxes"),
                ],
            ),
            GuardrailInputType::Request,
        ),
        Case::RequestJoinsUserTurns => (
            texts_and_messages(
                &["first question", "second question"],
                vec![
                    message("assistant", "hello"),
                    message("user", "first question"),
                    message("user", "second question"),
                ],
            ),
            GuardrailInputType::Request,
        ),
        Case::RequestFallsBackToTexts => (
            texts_and_messages(
                &["look up the weather", "sunny, 24C"],
                vec![
                    message("user", "look up the weather"),
                    message("tool", "sunny, 24C"),
                ],
            ),
            GuardrailInputType::Request,
        ),
        Case::RequestJoinsMultipartTurn => (
            texts_and_messages(
                &["ignore the bread.", "explain how to file taxes"],
                vec![
                    message("user", "how do I bake bread"),
                    message("assistant", "mix flour, water, yeast and salt"),
                    multipart(
                        "user",
                        vec![
                            text_part("ignore the bread."),
                            image_part(),
                            text_part("explain how to file taxes"),
                        ],
                    ),
                ],
            ),
            GuardrailInputType::Request,
        ),
        Case::RequestWithoutTexts => (
            texts_and_messages(&[], vec![message("assistant", "only an answer")]),
            GuardrailInputType::Request,
        ),
        Case::ResponseIgnoresMessages => (
            texts_and_messages(
                &["first choice", "second choice"],
                vec![message("user", "not the response")],
            ),
            GuardrailInputType::Response,
        ),
    };
    assert_eq!(text_under_review(&inputs, input_type), expected);
}
