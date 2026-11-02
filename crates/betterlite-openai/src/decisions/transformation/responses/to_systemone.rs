use indexmap::IndexMap;
use serde_json::Number;

use betterlite_models::decisions::{DecisionsJson, Extra, HiddenParams, ir, systemone};

fn probability_map<Item, Value, Key, Val>(
    probabilities: &[Item],
    key: Key,
    value: Val,
) -> IndexMap<String, Value>
where
    Key: Fn(&Item) -> String,
    Val: Fn(&Item) -> Value,
{
    probabilities
        .iter()
        .map(|item| (key(item), value(item)))
        .collect()
}

macro_rules! systemone_decision_answer {
    ($fn_name:ident, $Variant:ident, $primary_field:ident: $PrimaryType:ty, $probabilities_field:ident: $ProbType:ty $(, $named_field:ident: $NamedType:ty)*) => {
        fn $fn_name(
            $primary_field: $PrimaryType,
            confidence: &Number,
            $probabilities_field: $ProbType,
            $($named_field: $NamedType,)*
            extra: &Extra,
        ) -> systemone::DecisionAnswer {
            systemone::DecisionAnswer::$Variant {
                $primary_field,
                confidence: confidence.clone(),
                $($named_field,)*
                $probabilities_field,
                extra: extra.clone(),
            }
        }
    };
}

systemone_decision_answer!(
    wrap_systemone_choice_answer,
    Choice,
    choice: String,
    probabilities: IndexMap<String, Number>
);
systemone_decision_answer!(
    wrap_systemone_score_answer,
    Score,
    score: Number,
    probabilities: IndexMap<String, Number>,
    legend: IndexMap<String, DecisionsJson>
);

macro_rules! ir_to_systemone_answer {
    ($fn_name:ident, $Variant:ident { $($field:ident),+ }, $build:expr) => {
        fn $fn_name(answer: &ir::DecisionAnswer) -> Option<systemone::DecisionAnswer> {
            let ir::DecisionAnswer::$Variant { $($field),+ } = answer else {
                return None;
            };
            Some($build)
        }
    };
}

ir_to_systemone_answer!(
    systemone_predicate_answer,
    Predicate { probability, extra },
    systemone::DecisionAnswer::Noul {
        noul: probability.clone(),
        extra: extra.clone(),
    }
);
ir_to_systemone_answer!(
    systemone_choice_answer,
    Choice {
        choice,
        confidence,
        probabilities,
        extra
    },
    wrap_systemone_choice_answer(
        choice.systemone_key().to_owned(),
        confidence,
        probability_map(
            probabilities,
            |item| item.value.systemone_key().to_owned(),
            |item| item.probability.clone(),
        ),
        extra,
    )
);
ir_to_systemone_answer!(
    systemone_score_answer,
    Score {
        score,
        confidence,
        probabilities,
        extra
    },
    wrap_systemone_score_answer(
        score.clone(),
        confidence,
        probability_map(
            probabilities,
            |item| item.value.to_string(),
            |item| { item.probability.clone() }
        ),
        probability_map(
            probabilities,
            |item| item.value.to_string(),
            |item| { item.label.clone() }
        ),
        extra,
    )
);

fn systemone_answer(answer: &ir::DecisionAnswer) -> Option<systemone::DecisionAnswer> {
    match answer {
        ir::DecisionAnswer::Predicate { .. } => systemone_predicate_answer(answer),
        ir::DecisionAnswer::Choice { .. } => systemone_choice_answer(answer),
        ir::DecisionAnswer::Score { .. } => systemone_score_answer(answer),
        ir::DecisionAnswer::Refusal => None,
    }
}

super::decision_response_conversion!(
    ir_to_systemone_response,
    response: ir::DecisionResponse,
    request,
    systemone::DecisionsResponse,
    keys,
    answers: keys
        .into_iter()
        .zip(response.answers.iter())
        .filter_map(|(key, answer)| systemone_answer(answer).map(|value| (key, value)))
        .collect(),
    usage: Some(systemone::DecisionsUsage {
        input_tokens: response.usage.input_tokens.clone(),
        output_tokens: response.usage.output_tokens.clone(),
        cached_tokens: response.usage.cached_tokens.clone(),
        cache_write_tokens: response.usage.cache_write_tokens.clone(),
        extra: response.usage.extra.clone(),
    }),
    hidden_params: HiddenParams::default()
);
