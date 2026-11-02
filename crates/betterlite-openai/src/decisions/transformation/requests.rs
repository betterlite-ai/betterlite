use indexmap::IndexMap;

use betterlite_models::decisions::openai::{ContentPart, InputMessage, MessageContent};
use betterlite_models::decisions::systemone::{self, QuestionName};
use betterlite_models::decisions::validators::{Unchecked, ValidationError};
use betterlite_models::decisions::{DecisionsJson, Extra, ir};

use super::questions::{self, SystemOneQuestionError};

const TEXT_ONLY_REASON: &str = "input_image content parts are not supported because System One providers accept text input only";

#[derive(Debug, thiserror::Error)]
pub enum RequestConversionError {
    #[error(transparent)]
    Unsupported(#[from] ir::UnsupportedRequest),
    #[error(transparent)]
    Question(#[from] SystemOneQuestionError),
    #[error(transparent)]
    Invalid(#[from] ValidationError),
}

#[must_use]
pub fn systemone_request_to_ir(request: &systemone::DecisionsRequestBody) -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: ir::DecisionInput::State(request.state.clone()),
        questions: request
            .questions
            .as_ref()
            .iter()
            .map(|(name, question)| questions::ir_question(name.as_ref().clone(), question))
            .collect(),
        safety_identifier: None,
    }
}

fn has_image(message: &InputMessage) -> bool {
    match &message.content {
        MessageContent::Text(_) => false,
        MessageContent::Parts(parts) => parts
            .iter()
            .any(|part| matches!(part, ContentPart::InputImage { .. })),
    }
}

fn message_text(message: &InputMessage) -> String {
    match &message.content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Parts(parts) => parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::InputText { text } => Some(text.as_str()),
                ContentPart::InputImage { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn systemone_state(input: &ir::DecisionInput) -> Result<DecisionsJson, ir::UnsupportedRequest> {
    match input {
        ir::DecisionInput::State(state) => Ok(state.clone()),
        ir::DecisionInput::Messages(messages) => {
            if messages.iter().any(has_image) {
                Err(ir::UnsupportedRequest {
                    reason: TEXT_ONLY_REASON.to_owned(),
                })
            } else {
                Ok(DecisionsJson::Text(
                    messages
                        .iter()
                        .map(message_text)
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                ))
            }
        }
    }
}

pub fn ir_to_systemone_request(
    model: &str,
    request: &ir::DecisionRequest,
) -> Result<systemone::DecisionsRequest, RequestConversionError> {
    let state = systemone_state(&request.input)?;
    let keys = questions::systemone_keys(&request.questions);
    let mut mapped: IndexMap<QuestionName, systemone::DecisionQuestion> = IndexMap::new();
    for (key, question) in keys.into_iter().zip(request.questions.iter()) {
        let name: QuestionName = Unchecked(key).try_into()?;
        mapped.insert(name, questions::systemone_question(question)?);
    }
    let questions = Unchecked(mapped).try_into()?;
    Ok(systemone::DecisionsRequest {
        model: model.to_owned(),
        body: systemone::DecisionsRequestBody {
            state,
            questions,
            extra: Extra::default(),
        },
    })
}
