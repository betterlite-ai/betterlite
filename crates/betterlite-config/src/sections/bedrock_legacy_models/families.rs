use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

// AI21's penalty blocks are free-form objects, kept in the class dictionary as given.
pub type Penalty = IndexMap<String, Json>;

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ai21Values {
    #[serde(rename = "maxTokens")]
    pub max_tokens: Option<i64>,
    pub temperature: Option<Number>,
    #[serde(rename = "topP")]
    pub top_p: Option<Number>,
    #[serde(rename = "stopSequences")]
    pub stop_sequences: Option<Vec<String>>,
    // LiteLLM's class attribute carries the "frequence" spelling
    #[serde(rename = "frequencePenalty")]
    pub frequence_penalty: Option<Penalty>,
    #[serde(rename = "presencePenalty")]
    pub presence_penalty: Option<Penalty>,
    #[serde(rename = "countPenalty")]
    pub count_penalty: Option<Penalty>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CohereValues {
    pub max_tokens: Option<i64>,
    pub temperature: Option<Number>,
    pub return_likelihood: Option<String>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct LlamaValues {
    #[serde(rename = "maxTokenCount")]
    pub max_token_count: Option<i64>,
    pub temperature: Option<Number>,
    #[serde(rename = "topP")]
    pub top_p: Option<Number>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MistralValues {
    pub max_tokens: Option<i64>,
    pub temperature: Option<Number>,
    pub top_p: Option<Number>,
    pub top_k: Option<Number>,
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TitanValues {
    #[serde(rename = "maxTokenCount")]
    pub max_token_count: Option<i64>,
    #[serde(rename = "stopSequences")]
    pub stop_sequences: Option<Vec<String>>,
    pub temperature: Option<Number>,
    #[serde(rename = "topP")]
    pub top_p: Option<Number>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnthropicValues {
    pub max_tokens_to_sample: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub temperature: Option<Number>,
    pub top_k: Option<Number>,
    pub top_p: Option<Number>,
    pub anthropic_version: Option<String>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct Resolved {
    pub ai21: Ai21Values,
    pub cohere: CohereValues,
    pub llama: LlamaValues,
    pub mistral: MistralValues,
    pub titan: TitanValues,
    pub anthropic: AnthropicValues,
}
