use betterlite_models::decisions::{Extra, Json};
use serde::Serialize;
use serde_json::Number;

use crate::gemini_embedding_records::error::GeminiEmbeddingError;
use crate::gemini_embedding_records::input::{
    EmbeddingInput, InputEntry, Part, ResolvedFiles, build_part, shape,
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
    let output_dimensionality = match dimensionality {
        None | Some(Json::Null) => None,
        Some(Json::Number(number)) if number.is_i64() || number.is_u64() => Some(number.clone()),
        Some(Json::Number(_) | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(GeminiEmbeddingError::Rejected {
                reason: String::from("the outputDimensionality value must be a whole number"),
            });
        }
    };
    let (task_type, title) = text_parameters(optional)?;
    Ok(EmbedParameters {
        output_dimensionality,
        task_type,
        title,
    })
}
fn text_parameters(
    optional: &Extra,
) -> Result<(Option<TaskType>, Option<String>), GeminiEmbeddingError> {
    let task = optional
        .get("task_type")
        .or_else(|| optional.get("taskType"));
    let task_type = match task {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => {
            Some(
                TaskType::parse(text).ok_or_else(|| GeminiEmbeddingError::Rejected {
                    reason: format!("{text} is not a Gemini embedding task type"),
                })?,
            )
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(GeminiEmbeddingError::Rejected {
                reason: String::from("the taskType value must be text"),
            });
        }
    };
    let title = match optional.get("title") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(GeminiEmbeddingError::Rejected {
                reason: String::from("the title value must be text"),
            });
        }
    };
    Ok((task_type, title))
}
pub fn transform_batch_request(
    input: &EmbeddingInput,
    model: &str,
    optional: &Extra,
    resolved_files: &ResolvedFiles,
) -> Result<BatchEmbeddingsRequest, GeminiEmbeddingError> {
    let parameters = filter_parameters(optional)?;
    let request_model = format!("models/{model}");
    let requests = contents(input, resolved_files)?
        .into_iter()
        .map(|content| EmbedContentRequest {
            model: Some(request_model.clone()),
            content,
            parameters: parameters.clone(),
        })
        .collect();
    Ok(BatchEmbeddingsRequest { requests })
}
pub fn transform_embed_content(
    input: &EmbeddingInput,
    optional: &Extra,
    resolved_files: &ResolvedFiles,
) -> Result<EmbedContentRequest, GeminiEmbeddingError> {
    Ok(EmbedContentRequest {
        model: None,
        content: Content {
            parts: flat_parts(input, resolved_files)?,
        },
        parameters: filter_parameters(optional)?,
    })
}
fn contents(
    input: &EmbeddingInput,
    resolved_files: &ResolvedFiles,
) -> Result<Vec<Content>, GeminiEmbeddingError> {
    let mut contents = Vec::new();
    match input {
        EmbeddingInput::Text(text) => contents.push(Content {
            parts: vec![build_part(text, resolved_files)?],
        }),
        EmbeddingInput::Entries(entries) => {
            for entry in entries {
                let parts = match entry {
                    InputEntry::Text(text) => vec![build_part(text, resolved_files)?],
                    InputEntry::Group(group) => group_parts(group, resolved_files)?,
                };
                contents.push(Content { parts });
            }
        }
    }
    Ok(contents)
}
fn flat_parts(
    input: &EmbeddingInput,
    resolved_files: &ResolvedFiles,
) -> Result<Vec<Part>, GeminiEmbeddingError> {
    if shape(input).nested {
        return Err(GeminiEmbeddingError::Rejected {
            reason: String::from("the embedContent path does not accept combined embedding groups"),
        });
    }
    Ok(contents(input, resolved_files)?
        .into_iter()
        .flat_map(|content| content.parts)
        .collect())
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
