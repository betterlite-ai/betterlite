use serde::{Deserialize, Serialize};

use crate::decisions::validators::{Bounded, Checked};
use crate::decisions::{Extra, HiddenParams, MAX_DECISION_QUESTIONS};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentPart {
    InputText {
        text: String,
    },
    InputImage {
        image_url: String,
        detail: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    User,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    #[default]
    Message,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputMessage {
    #[serde(default)]
    pub role: Role,
    #[serde(default, rename = "type")]
    pub kind: MessageKind,
    pub content: MessageContent,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DecisionInput {
    Text(String),
    Messages(Vec<InputMessage>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateQuestion {
    pub name: Option<String>,
    pub instructions: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ChoiceValue {
    Flag(bool),
    Text(String),
}

impl ChoiceValue {
    #[must_use]
    pub fn systemone_key(&self) -> &str {
        match self {
            Self::Flag(true) => "true",
            Self::Flag(false) => "false",
            Self::Text(text) => text,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: ChoiceValue,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceQuestion {
    pub name: Option<String>,
    pub instructions: String,
    pub choices: Bounded<Vec<ChoiceOption>, 2, 255>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreLevel {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreQuestion {
    pub name: Option<String>,
    pub instructions: String,
    pub levels: Bounded<Vec<ScoreLevel>, 2, 10>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionQuestion {
    Predicate(PredicateQuestion),
    Choice(Checked<ChoiceQuestion>),
    Score(ScoreQuestion),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DecisionRequestBody {
    pub input: DecisionInput,
    pub questions: Bounded<Vec<DecisionQuestion>, 1, MAX_DECISION_QUESTIONS>,
    pub safety_identifier: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DecisionRequest {
    pub model: String,
    #[serde(flatten)]
    pub body: DecisionRequestBody,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ChoiceProbability {
    pub value: ChoiceValue,
    pub probability: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ScoreProbability {
    pub value: u64,
    pub label: String,
    pub probability: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionAnswer {
    Predicate {
        name: Option<String>,
        probability: f64,
    },
    Choice {
        name: Option<String>,
        choice: ChoiceValue,
        probabilities: Vec<ChoiceProbability>,
        confidence: f64,
    },
    Score {
        name: Option<String>,
        score: f64,
        probabilities: Vec<ScoreProbability>,
        confidence: f64,
    },
    Refusal {
        name: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct InputTokensDetails {
    pub cached_tokens: u64,
    pub cache_write_tokens: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct OutputTokensDetails {
    pub reasoning_tokens: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DecisionUsage {
    pub input_tokens: u64,
    #[serde(default)]
    pub input_tokens_details: InputTokensDetails,
    pub output_tokens: u64,
    #[serde(default)]
    pub output_tokens_details: OutputTokensDetails,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct DecisionResponse {
    pub model: String,
    pub answers: Vec<DecisionAnswer>,
    pub usage: DecisionUsage,
    #[serde(skip)]
    pub hidden_params: HiddenParams,
    #[serde(flatten)]
    pub extra: Extra,
}
