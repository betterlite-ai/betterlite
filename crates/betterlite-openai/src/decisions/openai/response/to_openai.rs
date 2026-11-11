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
        ir::DecisionQuestion::Predicate(predicate) => Ok(predicate_answer(predicate, answer)),
        ir::DecisionQuestion::Choice(choice) => choice_answer(choice, answer),
        ir::DecisionQuestion::Score(score) => score_answer(score, answer),
    }
}

fn predicate_answer(
    question: &ir::PredicateQuestion,
    answer: &ir::DecisionAnswer,
) -> openai::DecisionAnswer {
    match answer {
        ir::DecisionAnswer::Predicate { probability, .. } => openai::DecisionAnswer::Predicate {
            name: question.name.clone(),
            probability: probability.clone(),
        },
        ir::DecisionAnswer::Choice { .. }
        | ir::DecisionAnswer::Score { .. }
        | ir::DecisionAnswer::Refusal => openai::DecisionAnswer::Refusal {
            name: question.name.clone(),
        },
    }
}

fn choice_answer(
    question: &ir::ChoiceQuestion,
    answer: &ir::DecisionAnswer,
) -> Result<openai::DecisionAnswer, ResponseConversionError> {
    let ir::DecisionAnswer::Choice {
        choice,
        confidence,
        probabilities,
        ..
    } = answer
    else {
        return Ok(openai::DecisionAnswer::Refusal {
            name: question.name.clone(),
        });
    };
    let mut keyed: IndexMap<&str, &Number> = IndexMap::new();
    for item in probabilities {
        keyed.insert(item.value.systemone_key(), &item.probability);
    }
    let mut output = Vec::with_capacity(question.choices.len());
    for option in &question.choices {
        let probability = match keyed.get(option.value.systemone_key()) {
            Some(probability) => (*probability).clone(),
            None => Number::from_f64(0.0).ok_or(ResponseConversionError::Probability)?,
        };
        output.push(openai::ChoiceProbability {
            value: option.value.clone(),
            probability,
        });
    }
    Ok(openai::DecisionAnswer::Choice {
        name: question.name.clone(),
        choice: choice.clone(),
        probabilities: output,
        confidence: confidence.clone(),
    })
}

fn score_answer(
    question: &ir::ScoreQuestion,
    answer: &ir::DecisionAnswer,
) -> Result<openai::DecisionAnswer, ResponseConversionError> {
    let ir::DecisionAnswer::Score {
        score,
        confidence,
        probabilities,
        ..
    } = answer
    else {
        return Ok(openai::DecisionAnswer::Refusal {
            name: question.name.clone(),
        });
    };
    let mut keyed: IndexMap<i64, &Number> = IndexMap::new();
    for item in probabilities {
        let value = item
            .value
            .as_i64()
            .ok_or(ResponseConversionError::LevelIndex)?;
        keyed.insert(value, &item.probability);
    }
    let mut output = Vec::with_capacity(question.levels.len());
    for (index, level) in question.levels.iter().enumerate() {
        let Ok(level_index) = i64::try_from(index) else {
            return Err(ResponseConversionError::LevelOverflow);
        };
        let probability = match keyed.get(&level_index) {
            Some(probability) => (*probability).clone(),
            None => Number::from_f64(0.0).ok_or(ResponseConversionError::Probability)?,
        };
        output.push(openai::ScoreProbability {
            value: Number::from(index),
            label: decisions_text(&level.label)?,
            probability,
        });
    }
    Ok(openai::DecisionAnswer::Score {
        name: question.name.clone(),
        score: score.clone(),
        probabilities: output,
        confidence: confidence.clone(),
    })
}
