use betterlite_config::sections::mock_response::MockResponse;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_core::mock::chat::{self, ChatAnswer};
use betterlite_core::rules::{RuleDecision, RuleError, RuleEvaluator, post_call, pre_call};
use betterlite_openai::chat::mock_failure;
use serde::Deserialize;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = RuleCall;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let evaluator = Decisions {
            rules: &input.rules,
        };
        let text = rule_input(input)?;
        match input.phase {
            Phase::Pre => {
                let content = text.as_deref().ok_or_else(missing_input)?;
                pre_call(&evaluator, content, &input.model)
                    .map_err(|error| rule_failure(&error))?;
            }
            Phase::Post => {
                post_call(&evaluator, text.as_deref(), &input.model)
                    .map_err(|error| rule_failure(&error))?;
            }
        }
        Ok(serde_json::Value::Bool(true))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleCall {
    phase: Phase,
    model: String,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    mock_response: Option<String>,
    rules: Vec<Option<Decision>>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Pre,
    Post,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Decision {
    Boolean(bool),
    Dict {
        decision: Option<bool>,
        message: Option<String>,
    },
}

struct Decisions<'rules> {
    rules: &'rules [Option<Decision>],
}

impl RuleEvaluator for Decisions<'_> {
    fn evaluate(&self, index: usize, _input: &str) -> Option<RuleDecision> {
        let rule = self.rules.get(index)?;
        Some(match rule {
            None => RuleDecision::NotCallable,
            Some(Decision::Boolean(value)) => RuleDecision::Boolean(*value),
            Some(Decision::Dict { decision, message }) => RuleDecision::Dict {
                decision: *decision,
                message: message.clone(),
            },
        })
    }
}

// the rules read the answer a mock call produces, so a text mock_response composes with B255's builder
fn rule_input(input: &RuleCall) -> Result<Option<String>, Box<crate::helpers::HelperError>> {
    let Some(text) = input.mock_response.as_ref() else {
        return Ok(input.input.clone());
    };
    let response = MockResponse::Text(text.clone());
    let plan = MockPlan {
        response: Some(&response),
        tool_calls: None,
        choice_count: None,
        prompt_tokens: None,
    };
    match chat::answer(&plan).map_err(mock_answer_failure)? {
        ChatAnswer::Content(content) => Ok(Some(content.to_owned())),
        ChatAnswer::Object(_) => Err(object_answer()),
    }
}

fn rule_failure(error: &RuleError) -> Box<crate::helpers::HelperError> {
    let metadata = error.classify().metadata();
    Box::new(crate::helpers::HelperError {
        class: String::from("APIResponseValidationError"),
        status: Some(metadata.status_code),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn mock_answer_failure(error: MockError) -> Box<crate::helpers::HelperError> {
    let facts = mock_failure(error).failure();
    Box::new(crate::helpers::HelperError {
        class: String::from("MockException"),
        status: Some(facts.status),
        error_type: None,
        param: None,
        code: None,
        message: String::from("the mock answer failed"),
    })
}

fn missing_input() -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("RuleInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: String::from("a pre-call rule needs an input"),
    })
}

fn object_answer() -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("RuleInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: String::from("a rule input needs a text mock_response"),
    })
}
