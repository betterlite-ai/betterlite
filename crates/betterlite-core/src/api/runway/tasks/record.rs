use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;

const MAX_TASK_BYTES: usize = 0x0100_0000;
const DEFAULT_FAILURE_MESSAGE: &str = "Unknown error";
const DEFAULT_FAILURE_CODE: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub enum TaskOutput {
    Url(String),
    Fields(BTreeMap<String, String>),
    Unsupported(String),
}

impl From<Box<RawValue>> for TaskOutput {
    fn from(raw: Box<RawValue>) -> Self {
        let text = raw.get();
        serde_json::from_str(text)
            .map(Self::Url)
            .or_else(|_| serde_json::from_str(text).map(Self::Fields))
            .unwrap_or_else(|_| Self::Unsupported(text.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub enum TaskOutputs {
    Text(String),
    Members(Vec<TaskOutput>),
    NotAList(String),
}

impl From<Box<RawValue>> for TaskOutputs {
    fn from(raw: Box<RawValue>) -> Self {
        let text = raw.get();
        if let Ok(output) = serde_json::from_str(text) {
            return Self::Text(output);
        }
        serde_json::from_str::<Vec<Box<RawValue>>>(text).map_or_else(
            |_| Self::NotAList(text.to_owned()),
            |members| Self::Members(members.into_iter().map(TaskOutput::from).collect()),
        )
    }
}

// a field the source reads with .get(): an absent key and a present null reach the projection differently
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub enum TaskValue {
    #[default]
    Missing,
    Present(Json),
}

impl From<Box<RawValue>> for TaskValue {
    fn from(raw: Box<RawValue>) -> Self {
        let Ok(value) = serde_json::from_str(raw.get()) else {
            return Self::Missing;
        };
        Self::Present(value)
    }
}

impl TaskValue {
    #[must_use]
    pub fn value(&self) -> Option<&Json> {
        match self {
            Self::Missing => None,
            Self::Present(value) => Some(value),
        }
    }

    // response_data.get(key, default): an absent key takes the default, and a present null stays null
    #[must_use]
    pub fn or_default(&self, fallback: &str) -> Json {
        match self {
            Self::Missing => Json::Text(fallback.to_owned()),
            Self::Present(value) => value.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunwayTask {
    pub id: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub failure: TaskValue,
    #[serde(rename = "failureCode", default)]
    pub failure_code: TaskValue,
    pub output: Option<TaskOutputs>,
    #[serde(rename = "createdAt", default)]
    pub created_at: TaskValue,
    #[serde(rename = "completedAt", default)]
    pub completed_at: TaskValue,
    pub progress: Option<f64>,
    #[serde(rename = "estimatedCost", default)]
    pub estimated_cost: TaskValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Succeeded,
    Running,
    Failed,
    Cancelled,
    Unknown(String),
}

impl RunwayTask {
    #[must_use]
    pub fn state(&self) -> TaskState {
        let status = self.status.as_deref().unwrap_or_default().to_uppercase();
        match status.as_str() {
            "SUCCEEDED" => TaskState::Succeeded,
            "PENDING" | "RUNNING" | "THROTTLED" => TaskState::Running,
            "FAILED" => TaskState::Failed,
            "CANCELLED" => TaskState::Cancelled,
            _ => TaskState::Unknown(status),
        }
    }

    #[must_use]
    pub fn failure_error(&self, kind: TaskKind) -> RunwayTaskError {
        RunwayTaskError::Failed {
            kind,
            reason: self
                .failure
                .value()
                .and_then(text_value)
                .map_or_else(|| DEFAULT_FAILURE_MESSAGE.to_owned(), str::to_owned),
            code: self
                .failure_code
                .value()
                .and_then(text_value)
                .map_or_else(|| DEFAULT_FAILURE_CODE.to_owned(), str::to_owned),
        }
    }

    // _estimated_cost_usd reads credits only when estimatedCost is a mapping; the projection keeps the numeric check
    #[must_use]
    pub fn estimated_credits(&self) -> Option<&Json> {
        let Json::Object(cost) = self.estimated_cost.value()? else {
            return None;
        };
        cost.get("credits")
    }
}

fn text_value(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub fn decode_runway_task(bytes: &[u8]) -> Result<RunwayTask, RunwayTaskError> {
    if bytes.len() > MAX_TASK_BYTES {
        return Err(RunwayTaskError::TooLarge {
            limit: MAX_TASK_BYTES,
        });
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let task = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| RunwayTaskError::Decode { source })?;
    decoder.end().map_err(RunwayTaskError::TrailingBytes)?;
    Ok(task)
}
