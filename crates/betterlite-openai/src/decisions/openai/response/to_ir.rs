use betterlite_models::decisions::openai;
use betterlite_models::decisions::{DecisionsJson, Extra, ir};

#[must_use]
pub fn openai_response_to_ir(
    response: &openai::DecisionResponse,
    request: &ir::DecisionRequest,
) -> ir::DecisionResponse {
    ir::DecisionResponse {
        model: Some(response.model.clone()),
        answers: request
            .questions
            .iter()
            .enumerate()
            .map(|(index, question)| ir_answer(question, response.answers.get(index)))
            .collect(),
        usage: ir::DecisionUsage {
            input_tokens: response.usage.input_tokens.clone(),
            output_tokens: response.usage.output_tokens.clone(),
            cached_tokens: response.usage.input_tokens_details.cached_tokens.clone(),
            cache_write_tokens: response
                .usage
                .input_tokens_details
                .cache_write_tokens
                .clone(),
            reasoning_tokens: response
                .usage
                .output_tokens_details
                .reasoning_tokens
                .clone(),
            extra: Extra::default(),
        },
        extra: Extra::default(),
    }
}

fn ir_answer(
    question: &ir::DecisionQuestion,
    answer: Option<&openai::DecisionAnswer>,
) -> ir::DecisionAnswer {
    match question {
        ir::DecisionQuestion::Predicate(_) => ir_predicate_answer(answer),
        ir::DecisionQuestion::Choice(_) => ir_choice_answer(answer),
        ir::DecisionQuestion::Score(score) => ir_score_answer(score, answer),
    }
}

fn ir_predicate_answer(answer: Option<&openai::DecisionAnswer>) -> ir::DecisionAnswer {
    match answer {
        Some(openai::DecisionAnswer::Predicate { probability, .. }) => {
            ir::DecisionAnswer::Predicate {
                probability: probability.clone(),
                extra: Extra::default(),
            }
        }
        Some(
            openai::DecisionAnswer::Choice { .. }
            | openai::DecisionAnswer::Score { .. }
            | openai::DecisionAnswer::Refusal { .. },
        )
        | None => ir::DecisionAnswer::Refusal,
    }
}

fn ir_choice_answer(answer: Option<&openai::DecisionAnswer>) -> ir::DecisionAnswer {
    match answer {
        Some(openai::DecisionAnswer::Choice {
            choice,
            confidence,
            probabilities,
            ..
        }) => ir::DecisionAnswer::Choice {
            choice: choice.clone(),
            confidence: confidence.clone(),
            probabilities: probabilities
                .iter()
                .map(|item| ir::ChoiceProbability {
                    value: item.value.clone(),
                    probability: item.probability.clone(),
                })
                .collect(),
            extra: Extra::default(),
        },
        Some(
            openai::DecisionAnswer::Predicate { .. }
            | openai::DecisionAnswer::Score { .. }
            | openai::DecisionAnswer::Refusal { .. },
        )
        | None => ir::DecisionAnswer::Refusal,
    }
}

fn ir_score_answer(
    question: &ir::ScoreQuestion,
    answer: Option<&openai::DecisionAnswer>,
) -> ir::DecisionAnswer {
    match answer {
        Some(openai::DecisionAnswer::Score {
            score,
            confidence,
            probabilities,
            ..
        }) => ir::DecisionAnswer::Score {
            score: score.clone(),
            confidence: confidence.clone(),
            probabilities: probabilities
                .iter()
                .map(|item| ir::ScoreProbability {
                    value: item.value.clone(),
                    label: level_label(question, item),
                    probability: item.probability.clone(),
                })
                .collect(),
            extra: Extra::default(),
        },
        Some(
            openai::DecisionAnswer::Predicate { .. }
            | openai::DecisionAnswer::Choice { .. }
            | openai::DecisionAnswer::Refusal { .. },
        )
        | None => ir::DecisionAnswer::Refusal,
    }
}

fn level_label(
    question: &ir::ScoreQuestion,
    probability: &openai::ScoreProbability,
) -> DecisionsJson {
    let index = probability
        .value
        .as_i64()
        .and_then(|value| usize::try_from(value).ok());
    index
        .and_then(|level| question.levels.get(level))
        .map_or_else(
            || DecisionsJson::Text(probability.label.clone()),
            |level| level.label.clone(),
        )
}
