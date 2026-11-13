use betterlite_models::decisions::ir;
use serde_json::{Value, json};

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
