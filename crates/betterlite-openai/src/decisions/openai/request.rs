pub mod wire;

use betterlite_models::decisions::openai::{
    self, ContentPart, DecisionInput, InputMessage, MessageContent,
};
use betterlite_models::decisions::systemone::NoulLabel;
use betterlite_models::decisions::text::{TextError, decisions_text};
use betterlite_models::decisions::{DecisionsJson, Extra, ir};

use self::wire::{
    ChoiceWire, ContentWire, InputWire, LevelWire, MessageWire, PartWire, QuestionWire, RequestWire,
};
use crate::decisions::transformation::requests::RequestConversionError;

pub(crate) const SINGLE_OPTION_REASON: &str =
    "OpenAI needs at least 2 choices or levels on every choice or score question";
const CHOICE_INSTRUCTIONS: &str = "Which choice best fits the input?";
const SCORE_INSTRUCTIONS: &str = "Which level best fits the input?";
const PREDICATE_INSTRUCTIONS: &str = "Is this true of the input?";

#[must_use]
pub fn openai_request_to_ir(request: &openai::DecisionRequestBody) -> ir::DecisionRequest {
    let instructions = |text: &str| Some(DecisionsJson::Text(text.to_owned()));
    let mut questions = Vec::new();
    for question in request.questions.as_ref() {
        let extra = Extra::default();
        questions.push(match question {
            openai::DecisionQuestion::Predicate(predicate) => {
                ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
                    name: predicate.name.clone(),
                    instructions: instructions(&predicate.instructions),
                    criteria: None,
                    extra,
                })
            }
            openai::DecisionQuestion::Choice(checked) => {
                let choice = checked.as_ref();
                ir::DecisionQuestion::Choice(ir::ChoiceQuestion {
                    name: choice.name.clone(),
                    instructions: instructions(&choice.instructions),
                    choices: ir_choices(choice),
                    extra,
                })
            }
            openai::DecisionQuestion::Score(score) => {
                ir::DecisionQuestion::Score(ir::ScoreQuestion {
                    name: score.name.clone(),
                    instructions: instructions(&score.instructions),
                    levels: ir_levels(score),
                    extra,
                })
            }
        });
    }
    ir::DecisionRequest {
        input: ir_input(&request.input),
        questions,
        safety_identifier: request.safety_identifier.clone(),
    }
}

fn ir_input(input: &DecisionInput) -> ir::DecisionInput {
    match input {
        DecisionInput::Text(state) => ir::DecisionInput::State(DecisionsJson::Text(state.clone())),
        DecisionInput::Messages(messages) => ir::DecisionInput::Messages(messages.clone()),
    }
}

fn ir_choices(choice: &openai::ChoiceQuestion) -> Vec<ir::ChoiceOption> {
    choice
        .choices
        .as_ref()
        .iter()
        .map(|option| ir::ChoiceOption {
            value: option.value.clone(),
            description: option.description.clone().map(DecisionsJson::Text),
        })
        .collect()
}

fn ir_levels(score: &openai::ScoreQuestion) -> Vec<ir::ScoreLevel> {
    score
        .levels
        .as_ref()
        .iter()
        .map(|level| ir::ScoreLevel {
            label: DecisionsJson::Text(level.label.clone()),
            description: level.description.clone(),
        })
        .collect()
}

pub fn ir_to_openai_request(
    model: &str,
    request: &ir::DecisionRequest,
) -> Result<RequestWire, RequestConversionError> {
    if request.questions.iter().any(has_one_option) {
        return Err(RequestConversionError::from(ir::UnsupportedRequest {
            reason: SINGLE_OPTION_REASON.to_owned(),
        }));
    }
    Ok(RequestWire {
        model: model.to_owned(),
        input: input_wire(&request.input)?,
        questions: request
            .questions
            .iter()
            .map(question_wire)
            .collect::<Result<_, _>>()?,
        safety_identifier: request.safety_identifier.clone(),
    })
}

const fn has_one_option(question: &ir::DecisionQuestion) -> bool {
    match question {
        ir::DecisionQuestion::Predicate(_) => false,
        ir::DecisionQuestion::Choice(choice) => choice.choices.len() < 2,
        ir::DecisionQuestion::Score(score) => score.levels.len() < 2,
    }
}

fn input_wire(input: &ir::DecisionInput) -> Result<InputWire, TextError> {
    match input {
        ir::DecisionInput::State(state) => Ok(InputWire::Text(decisions_text(state)?)),
        ir::DecisionInput::Messages(messages) => Ok(InputWire::Messages(
            messages.iter().map(message_wire).collect(),
        )),
    }
}

fn message_wire(message: &InputMessage) -> MessageWire {
    MessageWire {
        role: message.role,
        kind: message.kind,
        content: match &message.content {
            MessageContent::Text(text) => ContentWire::Text(text.clone()),
            MessageContent::Parts(parts) => ContentWire::Parts(
                parts
                    .iter()
                    .map(|part| match part {
                        ContentPart::InputText { text } => {
                            PartWire::InputText { text: text.clone() }
                        }
                        ContentPart::InputImage { image_url, detail } => PartWire::InputImage {
                            image_url: image_url.clone(),
                            detail: detail.clone(),
                        },
                    })
                    .collect(),
            ),
        },
    }
}

fn question_wire(question: &ir::DecisionQuestion) -> Result<QuestionWire, TextError> {
    match question {
        ir::DecisionQuestion::Choice(choice) => Ok(QuestionWire::Choice {
            name: choice.name.clone(),
            instructions: choice
                .instructions
                .as_ref()
                .map_or_else(|| Ok(CHOICE_INSTRUCTIONS.to_owned()), decisions_text)?,
            choices: choice
                .choices
                .iter()
                .map(option_wire)
                .collect::<Result<_, _>>()?,
        }),
        ir::DecisionQuestion::Score(score) => Ok(QuestionWire::Score {
            name: score.name.clone(),
            instructions: score
                .instructions
                .as_ref()
                .map_or_else(|| Ok(SCORE_INSTRUCTIONS.to_owned()), decisions_text)?,
            levels: score
                .levels
                .iter()
                .map(level_wire)
                .collect::<Result<_, _>>()?,
        }),
        ir::DecisionQuestion::Predicate(predicate) => {
            let instructions = predicate_instructions(predicate)?;
            Ok(QuestionWire::Predicate {
                name: predicate.name.clone(),
                instructions: if instructions.is_empty() {
                    PREDICATE_INSTRUCTIONS.to_owned()
                } else {
                    instructions
                },
            })
        }
    }
}

fn option_wire(option: &ir::ChoiceOption) -> Result<ChoiceWire, TextError> {
    Ok(ChoiceWire {
        value: option.value.clone(),
        description: option
            .description
            .as_ref()
            .map(decisions_text)
            .transpose()?,
    })
}

fn level_wire(level: &ir::ScoreLevel) -> Result<LevelWire, TextError> {
    Ok(LevelWire {
        label: decisions_text(&level.label)?,
        description: level.description.clone(),
    })
}

fn predicate_instructions(question: &ir::PredicateQuestion) -> Result<String, TextError> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(instructions) = &question.instructions {
        parts.push(decisions_text(instructions)?);
    }
    if let Some(criteria) = &question.criteria {
        for (label, candidate) in criteria {
            let Some(rule) = candidate else {
                continue;
            };
            let answer = match label {
                NoulLabel::True => "true",
                NoulLabel::False => "false",
            };
            parts.push(format!("Answer {answer} when: {}", decisions_text(rule)?));
        }
    }
    Ok(parts.join("\n\n"))
}
