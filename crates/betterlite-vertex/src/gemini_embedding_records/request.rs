use betterlite_models::decisions::{Extra, Json};
use serde::Serialize;
use serde_json::Number;

use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::input::{
    EmbeddingInput, InputEntry, Part, ResolvedFiles, build_part,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskType {
    TaskTypeUnspecified,
    RetrievalQuery,
    RetrievalDocument,
    SemanticSimilarity,
    Classification,
    Clustering,
    QuestionAnswering,
    FactVerification,
}

impl TaskType {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "TASK_TYPE_UNSPECIFIED" => Some(Self::TaskTypeUnspecified),
            "RETRIEVAL_QUERY" => Some(Self::RetrievalQuery),
            "RETRIEVAL_DOCUMENT" => Some(Self::RetrievalDocument),
            "SEMANTIC_SIMILARITY" => Some(Self::SemanticSimilarity),
            "CLASSIFICATION" => Some(Self::Classification),
            "CLUSTERING" => Some(Self::Clustering),
            "QUESTION_ANSWERING" => Some(Self::QuestionAnswering),
            "FACT_VERIFICATION" => Some(Self::FactVerification),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EmbedParameters {
    #[serde(
        rename = "outputDimensionality",
        skip_serializing_if = "Option::is_none"
    )]
    pub output_dimensionality: Option<Number>,
    #[serde(rename = "taskType", skip_serializing_if = "Option::is_none")]
    pub task_type: Option<TaskType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Content {
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmbedContentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub content: Content,
    #[serde(flatten)]
    pub parameters: EmbedParameters,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BatchEmbeddingsRequest {
    pub requests: Vec<EmbedContentRequest>,
}

pub fn filter_parameters(optional: &Extra) -> Result<EmbedParameters, GeminiEmbeddingError> {
    let dimensionality = optional
        .get("dimensions")
        .or_else(|| optional.get("outputDimensionality"));
    let task_type = optional
        .get("task_type")
        .or_else(|| optional.get("taskType"));
    Ok(EmbedParameters {
        output_dimensionality: number_parameter(dimensionality)?,
        task_type: task_parameter(task_type)?,
        title: text_parameter(optional.get("title"))?,
    })
}

fn number_parameter(value: Option<&Json>) -> Result<Option<Number>, GeminiEmbeddingError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) if number.is_i64() || number.is_u64() => {
            Ok(Some(number.clone()))
        }
        Some(Json::Number(_) | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(GeminiEmbeddingError::WrongType {
                field: "outputDimensionality",
                expected: "a whole number",
            })
        }
    }
}

fn task_parameter(value: Option<&Json>) -> Result<Option<TaskType>, GeminiEmbeddingError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => {
            TaskType::parse(text)
                .map(Some)
                .ok_or_else(|| GeminiEmbeddingError::Rejected {
                    reason: format!("{text} is not a Gemini embedding task type"),
                })
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GeminiEmbeddingError::WrongType {
                field: "taskType",
                expected: "text",
            })
        }
    }
}

fn text_parameter(value: Option<&Json>) -> Result<Option<String>, GeminiEmbeddingError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GeminiEmbeddingError::WrongType {
                field: "title",
                expected: "text",
            })
        }
    }
}

pub fn transform_batch_request(
    input: &EmbeddingInput,
    model: &str,
    optional: &Extra,
    resolved_files: &ResolvedFiles,
) -> Result<BatchEmbeddingsRequest, GeminiEmbeddingError> {
    let parameters = filter_parameters(optional)?;
    let request_model = format!("models/{model}");
    let mut requests = Vec::new();
    match input {
        EmbeddingInput::Text(text) => requests.push(EmbedContentRequest {
            model: Some(request_model),
            content: Content {
                parts: vec![build_part(text, resolved_files)?],
            },
            parameters,
        }),
        EmbeddingInput::Entries(entries) => {
            for entry in entries {
                let parts = match entry {
                    InputEntry::Text(text) => vec![build_part(text, resolved_files)?],
                    InputEntry::Group(group) => group_parts(group, resolved_files)?,
                };
                requests.push(EmbedContentRequest {
                    model: Some(request_model.clone()),
                    content: Content { parts },
                    parameters: parameters.clone(),
                });
            }
        }
    }
    Ok(BatchEmbeddingsRequest { requests })
}

pub fn transform_embed_content(
    input: &EmbeddingInput,
    optional: &Extra,
    resolved_files: &ResolvedFiles,
) -> Result<EmbedContentRequest, GeminiEmbeddingError> {
    let parameters = filter_parameters(optional)?;
    let mut parts = Vec::new();
    match input {
        EmbeddingInput::Text(text) => parts.push(build_part(text, resolved_files)?),
        EmbeddingInput::Entries(entries) => {
            for entry in entries {
                match entry {
                    InputEntry::Text(text) => parts.push(build_part(text, resolved_files)?),
                    InputEntry::Group(_) => {
                        return Err(GeminiEmbeddingError::Rejected {
                            reason: String::from(
                                "the embedContent path does not accept combined embedding groups",
                            ),
                        });
                    }
                }
            }
        }
    }
    Ok(EmbedContentRequest {
        model: None,
        content: Content { parts },
        parameters,
    })
}

fn group_parts(
    group: &[String],
    resolved_files: &ResolvedFiles,
) -> Result<Vec<Part>, GeminiEmbeddingError> {
    if group.is_empty() {
        return Err(GeminiEmbeddingError::Rejected {
            reason: String::from("a combined embedding group must hold at least one text"),
        });
    }
    let mut parts = Vec::with_capacity(group.len());
    for element in group {
        parts.push(build_part(element, resolved_files)?);
    }
    Ok(parts)
}
