use betterlite_openai::embedding::EmbeddingResponse;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::rust::double_option;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SdkResult {
    Success { value: ChatCompletion },
    Embedding { value: EmbeddingOutcomes },
    Vectors { value: QueryVectors },
    Failure(Failure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingOutcomes {
    pub sync: EmbeddingResponse,
    #[serde(rename = "async")]
    pub asynchronous: EmbeddingResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryVectors {
    pub sync: Vec<Number>,
    #[serde(rename = "async")]
    pub asynchronous: Vec<Number>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatCompletion {
    pub id: String,
    pub created: u64,
    pub object: String,
    pub model: Option<String>,
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub content: Option<String>,
    pub finish_reason: Option<String>,
}

// None: key absent, Some(None): JSON null, Some(Some(text)): text
pub type Nullable = Option<Option<String>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub class: String,
    pub status: u16,
    #[serde(
        default,
        rename = "type",
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub kind: Nullable,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub param: Nullable,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub code: Nullable,
    pub message: String,
}
