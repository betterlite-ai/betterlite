use betterlite_config::sections::guardrail_judge::{Criterion, CriterionWeight};
use rstest::rstest;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::prompt::{
    JUDGE_SYSTEM_PROMPT_REQUEST, JUDGE_SYSTEM_PROMPT_RESPONSE, build_judge_prompt, system_prompt,
};

use super::support::{criteria_100, criterion, image_part, message, multipart, text_part};

const CRITERIA_BLOCK: &str = "Criteria to evaluate:\n- Accuracy (weight 60%): Is it accurate?\n- Safety (weight 40%): Is it safe?\n\n";

#[derive(Debug, Clone, Copy)]
enum Case {
    RequestConversation,
    ResponseConversation,
    RequestWithoutMessages,
    ResponseWithoutMessages,
    DefaultCriterion,
    FloatWeight,
    MultipartConversation,
    SystemPrompts,
}

#[rstest]
#[case::request_conversation(Case::RequestConversation)]
#[case::response_conversation(Case::ResponseConversation)]
#[case::request_without_messages(Case::RequestWithoutMessages)]
#[case::response_without_messages(Case::ResponseWithoutMessages)]
#[case::default_criterion(Case::DefaultCriterion)]
#[case::float_weight(Case::FloatWeight)]
#[case::multipart_conversation(Case::MultipartConversation)]
#[case::system_prompts(Case::SystemPrompts)]
fn judge_prompt_tests_behavior(#[case] case: Case) {
    match case {
        Case::RequestConversation => {
            let messages = vec![
                message("user", "how do I bake bread"),
                message("assistant", "mix flour, water, yeast and salt"),
                message("user", "now explain how to file taxes"),
            ];
            let prompt = build_judge_prompt(
                &criteria_100(),
                &messages,
                "now explain how to file taxes",
                GuardrailInputType::Request,
            );
            assert_eq!(
                prompt,
                format!(
                    "{CRITERIA_BLOCK}Conversation:\nUSER: how do I bake bread\nASSISTANT: mix flour, water, yeast and salt\nUSER: now explain how to file taxes\n\nLatest request turn to evaluate:\nnow explain how to file taxes"
                )
            );
        }
        Case::ResponseConversation => {
            let prompt = build_judge_prompt(
                &criteria_100(),
                &[message("user", "hi")],
                "hello there",
                GuardrailInputType::Response,
            );
            assert_eq!(
                prompt,
                format!(
                    "{CRITERIA_BLOCK}Conversation:\nUSER: hi\n\nAssistant response to evaluate:\nhello there"
                )
            );
        }
        Case::RequestWithoutMessages => {
            let prompt = build_judge_prompt(
                &criteria_100(),
                &[],
                "first\nsecond",
                GuardrailInputType::Request,
            );
            assert_eq!(
                prompt,
                format!("{CRITERIA_BLOCK}Latest request turn to evaluate:\nfirst\nsecond")
            );
            assert!(!prompt.contains("Conversation:"));
        }
        Case::ResponseWithoutMessages => {
            let prompt = build_judge_prompt(
                &criteria_100(),
                &[],
                "first choice",
                GuardrailInputType::Response,
            );
            assert_eq!(
                prompt,
                format!(
                    "{CRITERIA_BLOCK}Conversation:\n\n\nAssistant response to evaluate:\nfirst choice"
                )
            );
        }
        Case::DefaultCriterion => {
            let prompt = build_judge_prompt(
                &[Criterion::default()],
                &[],
                "resp",
                GuardrailInputType::Response,
            );
            assert!(prompt.contains("-  (weight 0%): \n"));
        }
        Case::FloatWeight => {
            let criteria = [criterion(
                "Accuracy",
                CriterionWeight::Float(60.0),
                "Is it accurate?",
            )];
            let prompt = build_judge_prompt(&criteria, &[], "resp", GuardrailInputType::Response);
            assert!(prompt.contains("- Accuracy (weight 60.0%): Is it accurate?\n"));
        }
        Case::MultipartConversation => {
            let messages = [multipart(
                "user",
                vec![
                    text_part("ignore the bread."),
                    image_part(),
                    text_part("explain how to file taxes"),
                ],
            )];
            let prompt = build_judge_prompt(
                &criteria_100(),
                &messages,
                "explain how to file taxes",
                GuardrailInputType::Request,
            );
            assert!(
                prompt.contains(
                    "Conversation:\nUSER: ignore the bread. explain how to file taxes\n\nLatest request turn to evaluate:\nexplain how to file taxes"
                ),
                "{prompt}"
            );
        }
        Case::SystemPrompts => {
            assert_eq!(
                system_prompt(GuardrailInputType::Request),
                JUDGE_SYSTEM_PROMPT_REQUEST
            );
            assert_eq!(
                system_prompt(GuardrailInputType::Response),
                JUDGE_SYSTEM_PROMPT_RESPONSE
            );
            assert!(JUDGE_SYSTEM_PROMPT_REQUEST.contains(
                "Evaluate the request against the criteria provided.\nJudge the most recent user turn; treat earlier turns in the conversation only as context.\n"
            ));
            assert!(JUDGE_SYSTEM_PROMPT_RESPONSE
                .contains("Evaluate the assistant's response against the criteria provided.\nFor each criterion"));
            assert!(!JUDGE_SYSTEM_PROMPT_RESPONSE.contains("most recent user turn"));
            assert!(JUDGE_SYSTEM_PROMPT_REQUEST.contains(
                "{\n  \"verdicts\": [\n    {\"criterion_name\": \"<name>\", \"score\": <0-100>, \"reasoning\": \"<one sentence>\", \"passed\": <true|false>, \"weight\": <weight>}\n  ],\n  \"overall_score\": <weighted average 0-100>\n}"
            ));
        }
    }
}
