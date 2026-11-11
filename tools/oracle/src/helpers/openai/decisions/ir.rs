use betterlite_models::decisions::systemone::NoulCriteria;
use betterlite_models::decisions::{DecisionsJson, Extra, ir};
use serde_json::{Value, json};

use super::input::{
    IrAnswer, IrChoiceOption, IrChoiceProbability, IrInput, IrQuestion, IrRequest, IrResponse,
    IrScoreLevel, IrScoreProbability,
};

pub(super) fn ir_request(request: &IrRequest) -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: match &request.input {
            IrInput::State { state } => ir::DecisionInput::State(state.clone()),
            IrInput::Messages { messages } => ir::DecisionInput::Messages(messages.clone()),
        },
        questions: request.questions.iter().map(ir_question).collect(),
        safety_identifier: request.safety_identifier.clone(),
    }
}

fn ir_question(question: &IrQuestion) -> ir::DecisionQuestion {
    match question {
        IrQuestion::Predicate {
            name,
            instructions,
            criteria,
            extra,
        } => ir::DecisionQuestion::Predicate(ir_predicate_question(
            name.as_ref(),
            instructions.as_ref(),
            criteria.as_ref(),
            extra,
        )),
        IrQuestion::Choice {
            name,
            instructions,
            choices,
            extra,
        } => ir::DecisionQuestion::Choice(ir_choice_question(
            name.as_ref(),
            instructions.as_ref(),
            choices,
            extra,
        )),
        IrQuestion::Score {
            name,
            instructions,
            levels,
            extra,
        } => ir::DecisionQuestion::Score(ir_score_question(
            name.as_ref(),
            instructions.as_ref(),
            levels,
            extra,
        )),
    }
}

fn ir_predicate_question(
    name: Option<&String>,
    instructions: Option<&DecisionsJson>,
    criteria: Option<&NoulCriteria>,
    extra: &Extra,
) -> ir::PredicateQuestion {
    ir::PredicateQuestion {
        name: name.cloned(),
        instructions: instructions.cloned(),
        criteria: criteria.cloned(),
        extra: extra.clone(),
    }
}

fn ir_choice_question(
    name: Option<&String>,
    instructions: Option<&DecisionsJson>,
    choices: &[IrChoiceOption],
    extra: &Extra,
) -> ir::ChoiceQuestion {
    ir::ChoiceQuestion {
        name: name.cloned(),
        instructions: instructions.cloned(),
        choices: choices
            .iter()
            .map(|option| ir::ChoiceOption {
                value: option.value.clone(),
                description: option.description.clone(),
            })
            .collect(),
        extra: extra.clone(),
    }
}

fn ir_score_question(
    name: Option<&String>,
    instructions: Option<&DecisionsJson>,
    levels: &[IrScoreLevel],
    extra: &Extra,
) -> ir::ScoreQuestion {
    ir::ScoreQuestion {
        name: name.cloned(),
        instructions: instructions.cloned(),
        levels: levels
            .iter()
            .map(|level| ir::ScoreLevel {
                label: level.label.clone(),
                description: level.description.clone(),
            })
            .collect(),
        extra: extra.clone(),
    }
}

pub(super) fn ir_response(response: &IrResponse) -> ir::DecisionResponse {
    ir::DecisionResponse {
        model: response.model.clone(),
        answers: response.answers.iter().map(ir_answer).collect(),
        usage: ir::DecisionUsage {
            input_tokens: response.usage.input_tokens.clone(),
            output_tokens: response.usage.output_tokens.clone(),
            cached_tokens: response.usage.cached_tokens.clone(),
            cache_write_tokens: response.usage.cache_write_tokens.clone(),
            reasoning_tokens: response.usage.reasoning_tokens.clone(),
            extra: response.usage.extra.clone(),
        },
        extra: response.extra.clone(),
    }
}

fn ir_answer(answer: &IrAnswer) -> ir::DecisionAnswer {
    match answer {
        IrAnswer::Predicate { probability, extra } => ir::DecisionAnswer::Predicate {
            probability: probability.clone(),
            extra: extra.clone(),
        },
        IrAnswer::Choice {
            choice,
            confidence,
            probabilities,
            extra,
        } => ir::DecisionAnswer::Choice {
            choice: choice.clone(),
            confidence: confidence.clone(),
            probabilities: choice_probabilities(probabilities),
            extra: extra.clone(),
        },
        IrAnswer::Score {
            score,
            confidence,
            probabilities,
            extra,
        } => ir::DecisionAnswer::Score {
            score: score.clone(),
            confidence: confidence.clone(),
            probabilities: score_probabilities(probabilities),
            extra: extra.clone(),
        },
        IrAnswer::Refusal => ir::DecisionAnswer::Refusal,
    }
}

fn choice_probabilities(probabilities: &[IrChoiceProbability]) -> Vec<ir::ChoiceProbability> {
    probabilities
        .iter()
        .map(|item| ir::ChoiceProbability {
            value: item.value.clone(),
            probability: item.probability.clone(),
        })
        .collect()
}

fn score_probabilities(probabilities: &[IrScoreProbability]) -> Vec<ir::ScoreProbability> {
    probabilities
        .iter()
        .map(|item| ir::ScoreProbability {
            value: item.value.clone(),
            label: item.label.clone(),
            probability: item.probability.clone(),
        })
        .collect()
}

pub(super) fn ir_response_json(response: &ir::DecisionResponse) -> Value {
    json!({
        "model": response.model,
        "answers": response.answers.iter().map(ir_answer_json).collect::<Vec<_>>(),
        "usage": {
            "input_tokens": response.usage.input_tokens.0,
            "output_tokens": response.usage.output_tokens.0,
            "cached_tokens": response.usage.cached_tokens.0,
            "cache_write_tokens": response.usage.cache_write_tokens.0,
            "reasoning_tokens": response.usage.reasoning_tokens.0,
            "extra": response.usage.extra,
        },
        "extra": response.extra,
    })
}

fn ir_answer_json(answer: &ir::DecisionAnswer) -> Value {
    match answer {
        ir::DecisionAnswer::Predicate { probability, extra } => {
            json!({"predicate": {"probability": probability, "extra": extra}})
        }
        ir::DecisionAnswer::Choice {
            choice,
            confidence,
            probabilities,
            extra,
        } => json!({"choice": {
            "choice": choice,
            "confidence": confidence,
            "probabilities": probabilities
                .iter()
                .map(|item| json!({"value": item.value, "probability": item.probability}))
                .collect::<Vec<_>>(),
            "extra": extra,
        }}),
        ir::DecisionAnswer::Score {
            score,
            confidence,
            probabilities,
            extra,
        } => json!({"score": {
            "score": score,
            "confidence": confidence,
            "probabilities": probabilities
                .iter()
                .map(|item| json!({"value": item.value, "label": item.label, "probability": item.probability}))
                .collect::<Vec<_>>(),
            "extra": extra,
        }}),
        ir::DecisionAnswer::Refusal => json!({"refusal": {}}),
    }
}
