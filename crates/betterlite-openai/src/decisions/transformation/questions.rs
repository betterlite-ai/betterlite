use std::collections::HashSet;

use betterlite_models::decisions::openai::ChoiceValue;
use betterlite_models::decisions::systemone;
use betterlite_models::decisions::text::{TextError, decisions_text};
use betterlite_models::decisions::validators::{Checked, Unchecked, ValidationError};
use betterlite_models::decisions::{DecisionsJson, ir};

#[derive(Debug, thiserror::Error)]
pub enum SystemOneQuestionError {
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Text(#[from] TextError),
}

fn question_name(question: &ir::DecisionQuestion) -> Option<&str> {
    match question {
        ir::DecisionQuestion::Predicate(predicate) => predicate.name.as_deref(),
        ir::DecisionQuestion::Choice(choice) => choice.name.as_deref(),
        ir::DecisionQuestion::Score(score) => score.name.as_deref(),
    }
}

#[must_use]
pub fn systemone_keys(questions: &[ir::DecisionQuestion]) -> Vec<String> {
    let names: Vec<&str> = questions.iter().filter_map(question_name).collect();
    let unique_names: HashSet<&&str> = names.iter().collect();
    if unique_names.len() == questions.len() {
        names.into_iter().map(str::to_owned).collect()
    } else {
        (0..questions.len())
            .map(|index| index.to_string())
            .collect()
    }
}

fn ir_predicate_question(name: String, noul: &systemone::NoulQuestion) -> ir::DecisionQuestion {
    ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
        name: Some(name),
        instructions: noul.instructions.clone(),
        criteria: noul.criteria.clone(),
        extra: noul.extra.clone(),
    })
}

fn ir_choice_question(name: String, choice: &systemone::ChoiceQuestion) -> ir::DecisionQuestion {
    ir::DecisionQuestion::Choice(ir::ChoiceQuestion {
        name: Some(name),
        instructions: choice.instructions.clone(),
        choices: choice
            .criteria
            .as_ref()
            .iter()
            .map(|(key, description)| ir::ChoiceOption {
                value: ChoiceValue::Text(key.clone()),
                description: description.clone(),
            })
            .collect(),
        extra: choice.extra.clone(),
    })
}

fn ir_score_question(name: String, score: &systemone::ScoreQuestion) -> ir::DecisionQuestion {
    ir::DecisionQuestion::Score(ir::ScoreQuestion {
        name: Some(name),
        instructions: score.instructions.clone(),
        levels: score
            .criteria
            .as_ref()
            .iter()
            .map(|criterion| ir::ScoreLevel {
                label: criterion.clone(),
                description: None,
            })
            .collect(),
        extra: score.extra.clone(),
    })
}

#[must_use]
pub fn ir_question(name: String, question: &systemone::DecisionQuestion) -> ir::DecisionQuestion {
    match question {
        systemone::DecisionQuestion::Noul(checked) => ir_predicate_question(name, checked.as_ref()),
        systemone::DecisionQuestion::Choice(choice) => ir_choice_question(name, choice),
        systemone::DecisionQuestion::Score(score) => ir_score_question(name, score),
    }
}

fn level_criterion(level: &ir::ScoreLevel) -> Result<DecisionsJson, TextError> {
    match &level.description {
        None => Ok(level.label.clone()),
        Some(description) => {
            let label = decisions_text(&level.label)?;
            Ok(DecisionsJson::Text(format!("{label}: {description}")))
        }
    }
}

fn systemone_predicate_question(
    predicate: &ir::PredicateQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    let noul = systemone::NoulQuestion {
        instructions: predicate.instructions.clone(),
        criteria: predicate.criteria.clone(),
        extra: predicate.extra.clone(),
    };
    let checked: Checked<systemone::NoulQuestion> = Unchecked(noul).try_into()?;
    Ok(systemone::DecisionQuestion::Noul(checked))
}

fn systemone_choice_question(
    choice: &ir::ChoiceQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    let criteria = choice
        .choices
        .iter()
        .map(|option| {
            (
                option.value.systemone_key().to_owned(),
                option.description.clone(),
            )
        })
        .collect();
    let bounded = Unchecked(criteria).try_into()?;
    Ok(systemone::DecisionQuestion::Choice(
        systemone::ChoiceQuestion {
            instructions: choice.instructions.clone(),
            criteria: bounded,
            extra: choice.extra.clone(),
        },
    ))
}

fn systemone_score_question(
    score: &ir::ScoreQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    let criteria = score
        .levels
        .iter()
        .map(level_criterion)
        .collect::<Result<Vec<_>, _>>()?;
    let bounded = Unchecked(criteria).try_into()?;
    Ok(systemone::DecisionQuestion::Score(
        systemone::ScoreQuestion {
            instructions: score.instructions.clone(),
            criteria: bounded,
            extra: score.extra.clone(),
        },
    ))
}

pub fn systemone_question(
    question: &ir::DecisionQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    match question {
        ir::DecisionQuestion::Predicate(predicate) => systemone_predicate_question(predicate),
        ir::DecisionQuestion::Choice(choice) => systemone_choice_question(choice),
        ir::DecisionQuestion::Score(score) => systemone_score_question(score),
    }
}
