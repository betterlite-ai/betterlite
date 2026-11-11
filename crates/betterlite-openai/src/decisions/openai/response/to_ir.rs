use betterlite_models::decisions::openai;
use betterlite_models::decisions::{DecisionsJson, Extra, ir};

macro_rules! ir_decision_answer {
    ($fn_name:ident, $question:ident: $Question:ty, $Answer:ident { $($field:ident),+ }, $build:expr) => {
        fn $fn_name(
            $question: &$Question,
            answer: Option<&openai::DecisionAnswer>,
        ) -> ir::DecisionAnswer {
            let Some(openai::DecisionAnswer::$Answer { $($field),+ , .. }) = answer else {
                return ir::DecisionAnswer::Refusal;
            };
            $build
        }
    };
}

ir_decision_answer!(
    ir_predicate_answer,
    _question: ir::PredicateQuestion,
    Predicate { probability },
    ir::DecisionAnswer::Predicate {
        probability: probability.clone(),
        extra: Extra::default(),
    }
);
ir_decision_answer!(
    ir_choice_answer,
    _question: ir::ChoiceQuestion,
    Choice { choice, confidence, probabilities },
    ir::DecisionAnswer::Choice {
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
    }
);
ir_decision_answer!(
    ir_score_answer,
    question: ir::ScoreQuestion,
    Score { score, confidence, probabilities },
    ir::DecisionAnswer::Score {
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
    }
);

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
        ir::DecisionQuestion::Predicate(predicate) => ir_predicate_answer(predicate, answer),
        ir::DecisionQuestion::Choice(choice) => ir_choice_answer(choice, answer),
        ir::DecisionQuestion::Score(score) => ir_score_answer(score, answer),
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
