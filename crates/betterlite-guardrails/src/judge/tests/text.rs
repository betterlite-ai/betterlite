use betterlite_prompts::arguments::json::{Json as ContentJson, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::prompt::text_under_review;

use super::support::{message, texts, texts_and_messages};

#[derive(Debug, Clone, Copy)]
enum Case {
    ResponseJoinsTexts,
    RequestUsesLatestUserTurn,
    RequestJoinsConsecutiveUserTurns,
    RequestFallsBackToTexts,
    RequestJoinsMultipartTurn,
    RequestWithoutTexts,
    ResponseIgnoresMessages,
}

fn text_part(text: &str) -> ContentPart {
    let mut fields = Object::new();
    fields.insert("text".to_owned(), ContentJson::Text(text.to_owned()));
    ContentPart {
        kind: "text".to_owned(),
        fields,
    }
}

fn image_part() -> ContentPart {
    let mut fields = Object::new();
    fields.insert(
        "image_url".to_owned(),
        ContentJson::Text("data:image/png;base64,AAAA".to_owned()),
    );
    ContentPart {
        kind: "image_url".to_owned(),
        fields,
    }
}

fn multipart_message(role: &str, parts: Vec<ContentPart>) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

#[rstest]
#[case::response_joins_texts(Case::ResponseJoinsTexts, "first\nsecond")]
#[case::request_latest_user_turn(Case::RequestUsesLatestUserTurn, "now explain how to file taxes")]
#[case::request_joins_consecutive_user_turns(
    Case::RequestJoinsConsecutiveUserTurns,
    "first question\nsecond question"
)]
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
        Case::RequestJoinsConsecutiveUserTurns => (
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
                    multipart_message(
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
