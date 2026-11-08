use betterlite_error::error::{ErrorClass, ErrorContext};
use thiserror::Error;

mod tests;

// LiteLLM raises this text for every rule that rejects by returning False
const REJECTION: &str = "LLM Response failed post-call-rule check";

// LiteLLM runs whichever callables config.yaml names; betterlite injects the evaluator that knows them
pub trait RuleEvaluator {
    // None means no rule is configured past the last one, so evaluation stops there
    fn evaluate(&self, index: usize, input: &str) -> Option<RuleDecision>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulePhase {
    PreCall,
    PostCall,
}

// the shapes a configured rule can answer with, so no caller has to interpret Python values
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDecision {
    NotCallable,
    Boolean(bool),
    Dict {
        decision: Option<bool>,
        message: Option<String>,
    },
}

#[derive(Debug, Error)]
pub enum RuleError {
    #[error("the response from {model} failed a rule check")]
    Rejected {
        phase: RulePhase,
        model: String,
        message: String,
    },
    #[error("a pre-call rule check needs the message buffer")]
    MissingInput,
}

impl RuleError {
    #[must_use]
    pub fn classify(&self) -> ErrorClass {
        match self {
            Self::Rejected { model, message, .. } => {
                ErrorClass::ApiResponseValidation(Box::new(ErrorContext {
                    message: message.clone(),
                    model: Some(model.clone()),
                    provider: Some(String::new()),
                    max_retries: None,
                    num_retries: None,
                    body_type: None,
                    body_param: None,
                    body_code: None,
                }))
            }
            Self::MissingInput => ErrorClass::InternalServer(Box::new(ErrorContext {
                message: self.to_string(),
                model: None,
                provider: None,
                max_retries: None,
                num_retries: None,
                body_type: None,
                body_param: None,
                body_code: None,
            })),
        }
    }
}

// pre_call_rules only fails on a literal False; post_call_rules reads a dict decision and its message
pub fn evaluate<E>(
    phase: RulePhase,
    evaluator: &E,
    input: Option<&str>,
    model: &str,
) -> Result<(), RuleError>
where
    E: RuleEvaluator,
{
    let Some(text) = input else {
        return match phase {
            RulePhase::PreCall => Err(RuleError::MissingInput),
            RulePhase::PostCall => Ok(()),
        };
    };
    let mut index = 0_usize;
    while let Some(answer) = evaluator.evaluate(index, text) {
        if let Some(message) = rejection(phase, &answer) {
            return Err(RuleError::Rejected {
                phase,
                model: model.to_owned(),
                message,
            });
        }
        index = index.saturating_add(1);
    }
    Ok(())
}

fn rejection(phase: RulePhase, answer: &RuleDecision) -> Option<String> {
    match answer {
        RuleDecision::NotCallable | RuleDecision::Boolean(true) => None,
        RuleDecision::Boolean(false) => Some(String::from(REJECTION)),
        RuleDecision::Dict { decision, message } => match phase {
            RulePhase::PreCall => None,
            RulePhase::PostCall => (decision == &Some(false))
                .then(|| message.clone().unwrap_or_else(|| String::from(REJECTION))),
        },
    }
}
