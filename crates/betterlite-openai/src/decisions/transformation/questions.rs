use std::collections::HashSet;

use indexmap::IndexMap;

use betterlite_models::decisions::openai::ChoiceValue;
use betterlite_models::decisions::systemone;
use betterlite_models::decisions::text::{TextError, decisions_text};
use betterlite_models::decisions::validators::{Bounded, Unchecked, ValidationError};
use betterlite_models::decisions::{DecisionsJson, Extra, ir};

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

fn choice_options(
    criteria: &Bounded<IndexMap<String, Option<DecisionsJson>>, 1, 255>,
) -> Vec<ir::ChoiceOption> {
    criteria
        .as_ref()
        .iter()
        .map(|(key, description)| ir::ChoiceOption {
            value: ChoiceValue::Text(key.clone()),
            description: description.clone(),
        })
        .collect()
}

fn score_levels(criteria: &Bounded<Vec<DecisionsJson>, 1, 10>) -> Vec<ir::ScoreLevel> {
    criteria
        .as_ref()
        .iter()
        .map(|criterion| ir::ScoreLevel {
            label: criterion.clone(),
            description: None,
        })
        .collect()
}

macro_rules! ir_variant_fn {
    ($fn_name:ident, $Variant:ident, $Struct:ident, $field:ident, $ValueType:ty) => {
        const fn $fn_name(
            name: String,
            instructions: Option<DecisionsJson>,
            value: $ValueType,
            extra: Extra,
        ) -> ir::DecisionQuestion {
            ir::DecisionQuestion::$Variant(ir::$Struct {
                name: Some(name),
                instructions,
                $field: value,
                extra,
            })
        }
    };
}

ir_variant_fn!(
    predicate_from_systemone,
    Predicate,
    PredicateQuestion,
    criteria,
    Option<systemone::NoulCriteria>
);
ir_variant_fn!(
    choice_from_systemone,
    Choice,
    ChoiceQuestion,
    choices,
    Vec<ir::ChoiceOption>
);
ir_variant_fn!(
    score_from_systemone,
    Score,
    ScoreQuestion,
    levels,
    Vec<ir::ScoreLevel>
);

#[must_use]
pub fn ir_question(name: String, question: &systemone::DecisionQuestion) -> ir::DecisionQuestion {
    match question {
        systemone::DecisionQuestion::Noul(checked) => predicate_from_systemone(
            name,
            checked.as_ref().instructions.clone(),
            checked.as_ref().criteria.clone(),
            checked.as_ref().extra.clone(),
        ),
        systemone::DecisionQuestion::Choice(choice) => choice_from_systemone(
            name,
            choice.instructions.clone(),
            choice_options(&choice.criteria),
            choice.extra.clone(),
        ),
        systemone::DecisionQuestion::Score(score) => score_from_systemone(
            name,
            score.instructions.clone(),
            score_levels(&score.criteria),
            score.extra.clone(),
        ),
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

fn choice_criteria_entries(choice: &ir::ChoiceQuestion) -> IndexMap<String, Option<DecisionsJson>> {
    choice
        .choices
        .iter()
        .map(|option| {
            (
                option.value.systemone_key().to_owned(),
                option.description.clone(),
            )
        })
        .collect()
}

macro_rules! systemone_struct_fn {
    ($fn_name:ident, $Struct:ident, $CriteriaType:ty) => {
        const fn $fn_name(
            instructions: Option<DecisionsJson>,
            criteria: $CriteriaType,
            extra: Extra,
        ) -> systemone::$Struct {
            systemone::$Struct {
                instructions,
                criteria,
                extra,
            }
        }
    };
}

systemone_struct_fn!(
    build_noul_question,
    NoulQuestion,
    Option<systemone::NoulCriteria>
);
systemone_struct_fn!(
    build_choice_question,
    ChoiceQuestion,
    Bounded<IndexMap<String, Option<DecisionsJson>>, 1, 255>
);
systemone_struct_fn!(
    build_score_question,
    ScoreQuestion,
    Bounded<Vec<DecisionsJson>, 1, 10>
);

fn predicate_to_systemone(
    predicate: &ir::PredicateQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    let built = build_noul_question(
        predicate.instructions.clone(),
        predicate.criteria.clone(),
        predicate.extra.clone(),
    );
    Ok(systemone::DecisionQuestion::Noul(
        Unchecked(built).try_into()?,
    ))
}

fn choice_to_systemone(
    choice: &ir::ChoiceQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    let criteria = Unchecked(choice_criteria_entries(choice)).try_into()?;
    let built = build_choice_question(choice.instructions.clone(), criteria, choice.extra.clone());
    Ok(systemone::DecisionQuestion::Choice(built))
}

fn score_to_systemone(
    score: &ir::ScoreQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    let entries: Vec<DecisionsJson> = score
        .levels
        .iter()
        .map(level_criterion)
        .collect::<Result<_, _>>()?;
    let criteria = Unchecked(entries).try_into()?;
    let built = build_score_question(score.instructions.clone(), criteria, score.extra.clone());
    Ok(systemone::DecisionQuestion::Score(built))
}

pub fn systemone_question(
    question: &ir::DecisionQuestion,
) -> Result<systemone::DecisionQuestion, SystemOneQuestionError> {
    match question {
        ir::DecisionQuestion::Predicate(predicate) => predicate_to_systemone(predicate),
        ir::DecisionQuestion::Choice(choice) => choice_to_systemone(choice),
        ir::DecisionQuestion::Score(score) => score_to_systemone(score),
    }
}
