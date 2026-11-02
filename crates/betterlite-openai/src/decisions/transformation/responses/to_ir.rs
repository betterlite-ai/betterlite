use indexmap::IndexMap;
use serde_json::Number;

use betterlite_models::decisions::openai::ChoiceValue;
use betterlite_models::decisions::systemone;
use betterlite_models::decisions::{Extra, TokenCount, ir};

fn choice_value(typed_values: &IndexMap<&str, &ChoiceValue>, key: &str) -> ChoiceValue {
    typed_values.get(key).map_or_else(
        || ChoiceValue::Text(key.to_owned()),
        |value| (*value).clone(),
    )
}

macro_rules! ir_decision_answer {
    ($fn_name:ident, $Variant:ident, $primary_field:ident: $PrimaryType:ty, $probabilities_field:ident: $ProbType:ty) => {
        fn $fn_name(
            $primary_field: $PrimaryType,
            confidence: &Number,
            $probabilities_field: $ProbType,
            extra: &Extra,
        ) -> ir::DecisionAnswer {
            ir::DecisionAnswer::$Variant {
                $primary_field,
                confidence: confidence.clone(),
                $probabilities_field,
                extra: extra.clone(),
            }
        }
    };
}

ir_decision_answer!(
    wrap_ir_choice_answer,
    Choice,
    choice: ChoiceValue,
    probabilities: Vec<ir::ChoiceProbability>
);
ir_decision_answer!(
    wrap_ir_score_answer,
    Score,
    score: Number,
    probabilities: Vec<ir::ScoreProbability>
);

macro_rules! systemone_to_ir_answer {
    ($fn_name:ident, $question:ident: $Question:ty, $Variant:ident { $($field:ident),+ }, $build:expr) => {
        fn $fn_name($question: &$Question, answer: Option<&systemone::DecisionAnswer>) -> ir::DecisionAnswer {
            let Some(systemone::DecisionAnswer::$Variant { $($field),+ }) = answer else {
                return ir::DecisionAnswer::Refusal;
            };
            $build
        }
    };
}

systemone_to_ir_answer!(
    ir_predicate_answer,
    _question: ir::PredicateQuestion,
    Noul { noul, extra },
    ir::DecisionAnswer::Predicate {
        probability: noul.clone(),
        extra: extra.clone(),
    }
);
systemone_to_ir_answer!(
    ir_choice_answer,
    question: ir::ChoiceQuestion,
    Choice {
        choice,
        confidence,
        probabilities,
        extra
    },
    {
        let typed_values: IndexMap<&str, &ChoiceValue> = question
            .choices
            .iter()
            .map(|option| (option.value.systemone_key(), &option.value))
            .collect();
        wrap_ir_choice_answer(
            choice_value(&typed_values, choice),
            confidence,
            probabilities
                .iter()
                .map(|(key, probability)| ir::ChoiceProbability {
                    value: choice_value(&typed_values, key),
                    probability: probability.clone(),
                })
                .collect(),
            extra,
        )
    }
);
systemone_to_ir_answer!(
    ir_score_answer,
    question: ir::ScoreQuestion,
    Score {
        score,
        confidence,
        legend,
        probabilities,
        extra
    },
    wrap_ir_score_answer(
        score.clone(),
        confidence,
        question
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
        extra,
    )
);

fn ir_answer(
    question: &ir::DecisionQuestion,
    answer: Option<&systemone::DecisionAnswer>,
) -> ir::DecisionAnswer {
    match question {
        ir::DecisionQuestion::Predicate(predicate) => ir_predicate_answer(predicate, answer),
        ir::DecisionQuestion::Choice(choice) => ir_choice_answer(choice, answer),
        ir::DecisionQuestion::Score(score) => ir_score_answer(score, answer),
    }
}

super::decision_response_conversion!(
    systemone_response_to_ir,
    response: systemone::DecisionsResponse,
    request,
    ir::DecisionResponse,
    keys,
    answers: keys
        .iter()
        .zip(request.questions.iter())
        .map(|(key, question)| ir_answer(question, response.answers.get(key)))
        .collect(),
    usage: {
        let usage = response.usage.clone().unwrap_or_default();
        ir::DecisionUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_tokens: usage.cached_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            reasoning_tokens: TokenCount::default(),
            extra: usage.extra,
        }
    }
);

pub fn parse_systemone_response(
    payload: &[u8],
    request: &ir::DecisionRequest,
) -> Result<ir::DecisionResponse, serde_json::Error> {
    serde_json::from_slice::<systemone::DecisionsResponse>(payload)
        .map(|response| systemone_response_to_ir(&response, request))
}
