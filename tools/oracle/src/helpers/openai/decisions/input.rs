use betterlite_models::decisions::openai;
use betterlite_models::decisions::systemone::NoulCriteria;
use betterlite_models::decisions::{DecisionsJson, Extra, TokenCount};
use serde::Deserialize;
use serde_json::Number;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum IrInput {
    State { state: DecisionsJson },
    Messages { messages: Vec<openai::InputMessage> },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum IrQuestion {
    Predicate {
        name: Option<String>,
        instructions: Option<DecisionsJson>,
        criteria: Option<NoulCriteria>,
        #[serde(default)]
        extra: Extra,
    },
    Choice {
        name: Option<String>,
        instructions: Option<DecisionsJson>,
        choices: Vec<IrChoiceOption>,
        #[serde(default)]
        extra: Extra,
    },
    Score {
        name: Option<String>,
        instructions: Option<DecisionsJson>,
        levels: Vec<IrScoreLevel>,
        #[serde(default)]
        extra: Extra,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IrChoiceOption {
    pub(super) value: openai::ChoiceValue,
    pub(super) description: Option<DecisionsJson>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IrScoreLevel {
    pub(super) label: DecisionsJson,
    pub(super) description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IrRequest {
    pub(super) input: IrInput,
    pub(super) questions: Vec<IrQuestion>,
    pub(super) safety_identifier: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IrResponse {
    pub(super) model: Option<String>,
    pub(super) answers: Vec<IrAnswer>,
    #[serde(default)]
    pub(super) usage: IrUsage,
    #[serde(default)]
    pub(super) extra: Extra,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IrUsage {
    #[serde(default)]
    pub(super) input_tokens: TokenCount,
    #[serde(default)]
    pub(super) output_tokens: TokenCount,
    #[serde(default)]
    pub(super) cached_tokens: TokenCount,
    #[serde(default)]
    pub(super) cache_write_tokens: TokenCount,
    #[serde(default)]
    pub(super) reasoning_tokens: TokenCount,
    #[serde(default)]
    pub(super) extra: Extra,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum IrAnswer {
    Predicate {
        probability: Number,
        #[serde(default)]
        extra: Extra,
    },
    Choice {
        choice: openai::ChoiceValue,
        confidence: Number,
        probabilities: Vec<IrChoiceProbability>,
        #[serde(default)]
        extra: Extra,
    },
    Score {
        score: Number,
        confidence: Number,
        probabilities: Vec<IrScoreProbability>,
        #[serde(default)]
        extra: Extra,
    },
    Refusal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IrChoiceProbability {
    pub(super) value: openai::ChoiceValue,
    pub(super) probability: Number,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IrScoreProbability {
    pub(super) value: Number,
    pub(super) label: DecisionsJson,
    pub(super) probability: Number,
}
