use indexmap::IndexMap;
use serde_json::Number;

use betterlite_models::decisions::openai;
use betterlite_models::decisions::text::decisions_text;
use betterlite_models::decisions::{Extra, HiddenParams, TokenCount, ir};

use super::ResponseConversionError;

pub fn ir_to_openai_response(
    response: &ir::DecisionResponse,
    request: &ir::DecisionRequest,
    requested_model: &str,
) -> Result<openai::DecisionResponse, ResponseConversionError> {
    if response.answers.len() != request.questions.len() {
        return Err(ResponseConversionError::AnswerCount {
            answers: response.answers.len(),
            questions: request.questions.len(),
        });
    }
    let answers = request
        .questions
        .iter()
        .zip(&response.answers)
        .map(|(question, answer)| openai_answer(question, answer))
        .collect::<Result<_, _>>()?;
    Ok(openai::DecisionResponse {
        model: response
            .model
            .clone()
            .unwrap_or_else(|| requested_model.to_owned()),
        answers,
        usage: openai::DecisionUsage {
            input_tokens: response.usage.input_tokens.clone(),
            input_tokens_details: openai::InputTokensDetails {
                cached_tokens: response.usage.cached_tokens.clone(),
                cache_write_tokens: response.usage.cache_write_tokens.clone(),
            },
            output_tokens: response.usage.output_tokens.clone(),
            output_tokens_details: openai::OutputTokensDetails {
                reasoning_tokens: response.usage.reasoning_tokens.clone(),
            },
            total_tokens: total_tokens(&response.usage)?,
        },
        hidden_params: HiddenParams::default(),
        extra: Extra::default(),
    })
}

fn total_tokens(usage: &ir::DecisionUsage) -> Result<TokenCount, ResponseConversionError> {
    let input = usage
        .input_tokens
        .0
        .as_i64()
        .ok_or(ResponseConversionError::TokenCount)?;
    let output = usage
        .output_tokens
        .0
        .as_i64()
        .ok_or(ResponseConversionError::TokenCount)?;
    let total = input
        .checked_add(output)
        .ok_or(ResponseConversionError::TokenCount)?;
    Ok(TokenCount(Number::from(total)))
}

fn openai_answer(
    question: &ir::DecisionQuestion,
    answer: &ir::DecisionAnswer,
) -> Result<openai::DecisionAnswer, ResponseConversionError> {
    match question {
        ir::DecisionQuestion::Predicate(predicate) => match answer {
            ir::DecisionAnswer::Predicate { probability, .. } => {
                Ok(openai::DecisionAnswer::Predicate {
                    name: predicate.name.clone(),
                    probability: probability.clone(),
                })
            }
            ir::DecisionAnswer::Choice { .. }
            | ir::DecisionAnswer::Score { .. }
            | ir::DecisionAnswer::Refusal => Ok(openai::DecisionAnswer::Refusal {
                name: predicate.name.clone(),
            }),
        },
        ir::DecisionQuestion::Choice(choice) => choice_answer(choice, answer),
        ir::DecisionQuestion::Score(score) => score_answer(score, answer),
    }
}

fn probability_or_zero(value: Option<&Number>) -> Result<Number, ResponseConversionError> {
    value.map_or_else(
        || Number::from_f64(0.0).ok_or(ResponseConversionError::Probability),
        |probability| Ok(probability.clone()),
    )
}

macro_rules! answer_fields {
    ($answer:expr, $Variant:ident { $($field:ident),+ }, $name:expr) => {
        let ir::DecisionAnswer::$Variant { $($field),+ , .. } = $answer else {
            return Ok(openai::DecisionAnswer::Refusal {
                name: $name.clone(),
            });
        };
    };
}

fn choice_answer(
    question: &ir::ChoiceQuestion,
    answer: &ir::DecisionAnswer,
) -> Result<openai::DecisionAnswer, ResponseConversionError> {
    answer_fields!(
        answer,
        Choice {
            choice,
            confidence,
            probabilities
        },
        question.name
    );
    let mut keyed: IndexMap<&str, &Number> = IndexMap::new();
    for item in probabilities {
        keyed.insert(item.value.systemone_key(), &item.probability);
    }
    let mapped = question
        .choices
        .iter()
        .map(|option| {
            let probability =
                probability_or_zero(keyed.get(option.value.systemone_key()).copied())?;
            Ok(openai::ChoiceProbability {
                value: option.value.clone(),
                probability,
            })
        })
        .collect::<Result<Vec<_>, ResponseConversionError>>()?;
    Ok(openai::DecisionAnswer::Choice {
        name: question.name.clone(),
        choice: choice.clone(),
        probabilities: mapped,
        confidence: confidence.clone(),
    })
}

fn score_answer(
    question: &ir::ScoreQuestion,
    answer: &ir::DecisionAnswer,
) -> Result<openai::DecisionAnswer, ResponseConversionError> {
    answer_fields!(
        answer,
        Score {
            score,
            confidence,
            probabilities
        },
        question.name
    );
    let mut by_level: Vec<Option<&Number>> = vec![None; question.levels.len()];
    for item in probabilities {
        let index = item
            .value
            .as_i64()
            .ok_or(ResponseConversionError::LevelIndex)?;
        let Ok(level_slot) = usize::try_from(index) else {
            continue;
        };
        let Some(slot) = by_level.get_mut(level_slot) else {
            continue;
        };
        *slot = Some(&item.probability);
    }
    let mut output = Vec::with_capacity(question.levels.len());
    for (index, level) in question.levels.iter().enumerate() {
        let slot = by_level.get(index).copied().flatten();
        output.push(openai::ScoreProbability {
            value: Number::from(index),
            label: decisions_text(&level.label)?,
            probability: probability_or_zero(slot)?,
        });
    }
    Ok(openai::DecisionAnswer::Score {
        name: question.name.clone(),
        score: score.clone(),
        probabilities: output,
        confidence: confidence.clone(),
    })
}
