use indexmap::IndexMap;
use serde_json::Number;

use betterlite_models::decisions::openai::ChoiceValue;
use betterlite_models::decisions::systemone;
use betterlite_models::decisions::{Extra, HiddenParams, TokenCount, ir};

use super::questions;

fn ir_predicate_answer(answer: Option<&systemone::DecisionAnswer>) -> ir::DecisionAnswer {
    match answer {
        Some(systemone::DecisionAnswer::Noul { noul, extra }) => ir::DecisionAnswer::Predicate {
            probability: noul.clone(),
            extra: extra.clone(),
        },
        _ => ir::DecisionAnswer::Refusal,
    }
}

fn ir_choice_answer(
    question: &ir::ChoiceQuestion,
    answer: Option<&systemone::DecisionAnswer>,
) -> ir::DecisionAnswer {
    match answer {
        Some(systemone::DecisionAnswer::Choice {
            choice,
            confidence,
            probabilities,
            extra,
        }) => {
            let typed_values: IndexMap<&str, &ChoiceValue> = question
                .choices
                .iter()
                .map(|option| (option.value.systemone_key(), &option.value))
                .collect();
            ir::DecisionAnswer::Choice {
                choice: typed_values.get(choice.as_str()).map_or_else(
                    || ChoiceValue::Text(choice.clone()),
                    |value| (*value).clone(),
                ),
                confidence: confidence.clone(),
                probabilities: probabilities
                    .iter()
                    .map(|(key, probability)| ir::ChoiceProbability {
                        value: typed_values.get(key.as_str()).map_or_else(
                            || ChoiceValue::Text(key.clone()),
                            |value| (*value).clone(),
                        ),
                        probability: probability.clone(),
                    })
                    .collect(),
                extra: extra.clone(),
            }
        }
        _ => ir::DecisionAnswer::Refusal,
    }
}

fn ir_score_answer(
    question: &ir::ScoreQuestion,
    answer: Option<&systemone::DecisionAnswer>,
) -> ir::DecisionAnswer {
    match answer {
        Some(systemone::DecisionAnswer::Score {
            score,
            confidence,
            legend,
            probabilities,
            extra,
        }) => ir::DecisionAnswer::Score {
            score: score.clone(),
            confidence: confidence.clone(),
            probabilities: question
                .levels
                .iter()
                .enumerate()
                .map(|(index, level)| {
                    let key = index.to_string();
                    ir::ScoreProbability {
                        value: Number::from(index),
                        label: legend
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| level.label.clone()),
                        probability: probabilities
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| Number::from(0_u8)),
                    }
                })
                .collect(),
            extra: extra.clone(),
        },
        _ => ir::DecisionAnswer::Refusal,
    }
}

fn ir_answer(
    question: &ir::DecisionQuestion,
    answer: Option<&systemone::DecisionAnswer>,
) -> ir::DecisionAnswer {
    match question {
        ir::DecisionQuestion::Predicate(_) => ir_predicate_answer(answer),
        ir::DecisionQuestion::Choice(choice) => ir_choice_answer(choice, answer),
        ir::DecisionQuestion::Score(score) => ir_score_answer(score, answer),
    }
}

#[must_use]
pub fn systemone_response_to_ir(
    response: &systemone::DecisionsResponse,
    request: &ir::DecisionRequest,
) -> ir::DecisionResponse {
    let usage = response.usage.clone().unwrap_or_default();
    let keys = questions::systemone_keys(&request.questions);
    ir::DecisionResponse {
        model: response.model.clone(),
        answers: keys
            .iter()
            .zip(request.questions.iter())
            .map(|(key, question)| ir_answer(question, response.answers.get(key)))
            .collect(),
        usage: ir::DecisionUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cached_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            reasoning_tokens: TokenCount::default(),
            extra: usage.extra,
        },
        extra: response.extra.clone(),
    }
}

pub fn parse_systemone_response(
    payload: &[u8],
    request: &ir::DecisionRequest,
) -> Result<ir::DecisionResponse, serde_json::Error> {
    serde_json::from_slice::<systemone::DecisionsResponse>(payload)
        .map(|response| systemone_response_to_ir(&response, request))
}

fn systemone_predicate_answer(probability: &Number, extra: &Extra) -> systemone::DecisionAnswer {
    systemone::DecisionAnswer::Noul {
        noul: probability.clone(),
        extra: extra.clone(),
    }
}

fn systemone_choice_answer(
    choice: &ChoiceValue,
    confidence: &Number,
    probabilities: &[ir::ChoiceProbability],
    extra: &Extra,
) -> systemone::DecisionAnswer {
    systemone::DecisionAnswer::Choice {
        choice: choice.systemone_key().to_owned(),
        confidence: confidence.clone(),
        probabilities: probabilities
            .iter()
            .map(|item| {
                (
                    item.value.systemone_key().to_owned(),
                    item.probability.clone(),
                )
            })
            .collect(),
        extra: extra.clone(),
    }
}

fn systemone_score_answer(
    score: &Number,
    confidence: &Number,
    probabilities: &[ir::ScoreProbability],
    extra: &Extra,
) -> systemone::DecisionAnswer {
    systemone::DecisionAnswer::Score {
        score: score.clone(),
        confidence: confidence.clone(),
        legend: probabilities
            .iter()
            .map(|item| (item.value.to_string(), item.label.clone()))
            .collect(),
        probabilities: probabilities
            .iter()
            .map(|item| (item.value.to_string(), item.probability.clone()))
            .collect(),
        extra: extra.clone(),
    }
}

fn systemone_answer(answer: &ir::DecisionAnswer) -> Option<systemone::DecisionAnswer> {
    match answer {
        ir::DecisionAnswer::Predicate { probability, extra } => {
            Some(systemone_predicate_answer(probability, extra))
        }
        ir::DecisionAnswer::Choice {
            choice,
            confidence,
            probabilities,
            extra,
        } => Some(systemone_choice_answer(
            choice,
            confidence,
            probabilities,
            extra,
        )),
        ir::DecisionAnswer::Score {
            score,
            confidence,
            probabilities,
            extra,
        } => Some(systemone_score_answer(
            score,
            confidence,
            probabilities,
            extra,
        )),
        ir::DecisionAnswer::Refusal => None,
    }
}

#[must_use]
pub fn ir_to_systemone_response(
    response: &ir::DecisionResponse,
    request: &ir::DecisionRequest,
) -> systemone::DecisionsResponse {
    let keys = questions::systemone_keys(&request.questions);
    let answers: IndexMap<String, systemone::DecisionAnswer> = keys
        .into_iter()
        .zip(response.answers.iter())
        .filter_map(|(key, answer)| systemone_answer(answer).map(|value| (key, value)))
        .collect();
    systemone::DecisionsResponse {
        model: response.model.clone(),
        answers,
        usage: Some(systemone::DecisionsUsage {
            input_tokens: response.usage.input_tokens.clone(),
            output_tokens: response.usage.output_tokens.clone(),
            cached_tokens: response.usage.cached_tokens.clone(),
            cache_write_tokens: response.usage.cache_write_tokens.clone(),
            extra: response.usage.extra.clone(),
        }),
        hidden_params: HiddenParams::default(),
        extra: response.extra.clone(),
    }
}
