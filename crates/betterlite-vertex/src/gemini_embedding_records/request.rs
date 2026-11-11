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
    pub model: String,
    pub content: Content,
    #[serde(flatten)]
    pub parameters: EmbedParameters,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BatchEmbeddingsRequest {
    pub requests: Vec<EmbedContentRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmbedContentBody {
    pub content: Content,
    #[serde(flatten)]
    pub parameters: EmbedParameters,
}

pub fn filter_parameters(optional: &Extra) -> Result<EmbedParameters, GeminiEmbeddingError> {
    let mut parameters = EmbedParameters {
        output_dimensionality: output_dimensionality(optional.get("outputDimensionality"))?,
        task_type: task_type(optional.get("taskType"), "taskType")?,
        title: title(optional.get("title"))?,
    };
    if optional.contains_key("dimensions") {
        parameters.output_dimensionality = output_dimensionality(optional.get("dimensions"))?;
    }
    if optional.contains_key("task_type") {
        parameters.task_type = task_type(optional.get("task_type"), "task_type")?;
    }
    Ok(parameters)
}

fn output_dimensionality(value: Option<&Json>) -> Result<Option<Number>, GeminiEmbeddingError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) if number.is_i64() || number.is_u64() => {
            Ok(Some(number.clone()))
        }
        Some(Json::Number(_) | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(GeminiEmbeddingError::NotInteger {
                field: "outputDimensionality",
            })
        }
    }
}

fn task_type(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<TaskType>, GeminiEmbeddingError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => {
            TaskType::parse(text)
                .map(Some)
                .ok_or_else(|| GeminiEmbeddingError::TaskType {
                    value: text.clone(),
                })
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GeminiEmbeddingError::NotText { field })
        }
    }
}

fn title(value: Option<&Json>) -> Result<Option<String>, GeminiEmbeddingError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GeminiEmbeddingError::NotText { field: "title" })
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
            model: request_model,
            content: Content {
                parts: vec![build_part(text, resolved_files)?],
            },
            parameters,
        }),
        EmbeddingInput::Entries(entries) => {
            let mut grouped = Vec::with_capacity(entries.len());
            for entry in entries {
                grouped.push(match entry {
                    InputEntry::Text(text) => vec![build_part(text, resolved_files)?],
                    InputEntry::Group(group) => group_parts(group, resolved_files)?,
                });
            }
            for parts in grouped {
                requests.push(EmbedContentRequest {
                    model: request_model.clone(),
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
) -> Result<EmbedContentBody, GeminiEmbeddingError> {
    let parameters = filter_parameters(optional)?;
    let mut parts = Vec::new();
    match input {
        EmbeddingInput::Text(text) => parts.push(build_part(text, resolved_files)?),
        EmbeddingInput::Entries(entries) => {
            for entry in entries {
                match entry {
                    InputEntry::Text(text) => parts.push(build_part(text, resolved_files)?),
                    InputEntry::Group(_) => {
                        return Err(GeminiEmbeddingError::NestedGroupUnsupported);
                    }
                }
            }
        }
    }
    Ok(EmbedContentBody {
        content: Content { parts },
        parameters,
    })
}

fn group_parts(
    group: &[String],
    resolved_files: &ResolvedFiles,
) -> Result<Vec<Part>, GeminiEmbeddingError> {
    if group.is_empty() {
        return Err(GeminiEmbeddingError::EmptyGroup);
    }
    let mut parts = Vec::with_capacity(group.len());
    for element in group {
        parts.push(build_part(element, resolved_files)?);
    }
    Ok(parts)
}
