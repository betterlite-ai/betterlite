use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::decisions::validators::{Bounded, Checked};
use crate::decisions::{DecisionsJson, Extra, HiddenParams, MAX_DECISION_QUESTIONS, TokenCount};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoulLabel {
    True,
    False,
}

pub type NoulCriteria = IndexMap<NoulLabel, Option<DecisionsJson>>;
pub type QuestionName = Bounded<String, 1, { usize::MAX }>;
pub type DecisionQuestionMap =
    Bounded<IndexMap<QuestionName, DecisionQuestion>, 1, MAX_DECISION_QUESTIONS>;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NoulQuestion {
    pub instructions: Option<DecisionsJson>,
    pub criteria: Option<NoulCriteria>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChoiceQuestion {
    pub instructions: Option<DecisionsJson>,
    pub criteria: Bounded<IndexMap<String, Option<DecisionsJson>>, 1, 255>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ScoreQuestion {
    pub instructions: Option<DecisionsJson>,
    pub criteria: Bounded<Vec<DecisionsJson>, 1, 10>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionQuestion {
    Noul(Checked<NoulQuestion>),
    Choice(ChoiceQuestion),
    Score(ScoreQuestion),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DecisionsRequestBody {
    pub state: DecisionsJson,
    pub questions: DecisionQuestionMap,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DecisionsRequest {
    pub model: String,
    #[serde(flatten)]
    pub body: DecisionsRequestBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DecisionAnswer {
    Noul {
        noul: Number,
        #[serde(flatten)]
        extra: Extra,
    },
    Choice {
        choice: String,
        confidence: Number,
        probabilities: IndexMap<String, Number>,
        #[serde(flatten)]
        extra: Extra,
    },
    Score {
        score: Number,
        confidence: Number,
        legend: IndexMap<String, DecisionsJson>,
        probabilities: IndexMap<String, Number>,
        #[serde(flatten)]
        extra: Extra,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct DecisionsUsage {
    pub input_tokens: TokenCount,
    pub output_tokens: TokenCount,
    #[serde(skip_serializing)]
    pub cached_tokens: TokenCount,
    #[serde(skip_serializing)]
    pub cache_write_tokens: TokenCount,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DecisionsResponse {
    pub model: Option<String>,
    pub answers: IndexMap<String, DecisionAnswer>,
    pub usage: Option<DecisionsUsage>,
    #[serde(skip)]
    pub hidden_params: HiddenParams,
    #[serde(flatten)]
    pub extra: Extra,
}
