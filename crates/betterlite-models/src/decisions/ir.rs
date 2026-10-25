use crate::decisions::openai::{ChoiceValue, InputMessage};
use crate::decisions::systemone::NoulCriteria;
use crate::decisions::{DecisionsJson, Extra};

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

#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceProbability {
    pub value: ChoiceValue,
    pub probability: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScoreProbability {
    pub value: u64,
    pub label: DecisionsJson,
    pub probability: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DecisionAnswer {
    Predicate {
        probability: f64,
        extra: Extra,
    },
    Choice {
        choice: ChoiceValue,
        confidence: f64,
        probabilities: Vec<ChoiceProbability>,
        extra: Extra,
    },
    Score {
        score: f64,
        confidence: f64,
        probabilities: Vec<ScoreProbability>,
        extra: Extra,
    },
    Refusal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecisionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq)]
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
