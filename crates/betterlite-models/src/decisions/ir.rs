use serde_json::Number;

use crate::decisions::openai::{ChoiceValue, InputMessage};
use crate::decisions::systemone::NoulCriteria;
use crate::decisions::{DecisionsJson, Extra, TokenCount};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionInput {
    State(DecisionsJson),
    Messages(Vec<InputMessage>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredicateQuestion {
    pub name: Option<String>,
    pub instructions: Option<DecisionsJson>,
    pub criteria: Option<NoulCriteria>,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    pub value: ChoiceValue,
    pub description: Option<DecisionsJson>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceQuestion {
    pub name: Option<String>,
    pub instructions: Option<DecisionsJson>,
    pub choices: Vec<ChoiceOption>,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreLevel {
    pub label: DecisionsJson,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreQuestion {
    pub name: Option<String>,
    pub instructions: Option<DecisionsJson>,
    pub levels: Vec<ScoreLevel>,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionQuestion {
    Predicate(PredicateQuestion),
    Choice(ChoiceQuestion),
    Score(ScoreQuestion),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionRequest {
    pub input: DecisionInput,
    pub questions: Vec<DecisionQuestion>,
    pub safety_identifier: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceProbability {
    pub value: ChoiceValue,
    pub probability: Number,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreProbability {
    pub value: Number,
    pub label: DecisionsJson,
    pub probability: Number,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionAnswer {
    Predicate {
        probability: Number,
        extra: Extra,
    },
    Choice {
        choice: ChoiceValue,
        confidence: Number,
        probabilities: Vec<ChoiceProbability>,
        extra: Extra,
    },
    Score {
        score: Number,
        confidence: Number,
        probabilities: Vec<ScoreProbability>,
        extra: Extra,
    },
    Refusal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionUsage {
    pub input_tokens: TokenCount,
    pub output_tokens: TokenCount,
    pub cached_tokens: TokenCount,
    pub cache_write_tokens: TokenCount,
    pub reasoning_tokens: TokenCount,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionResponse {
    pub model: Option<String>,
    pub answers: Vec<DecisionAnswer>,
    pub usage: DecisionUsage,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct UnsupportedRequest {
    pub reason: String,
}
