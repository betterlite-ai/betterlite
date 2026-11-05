use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;

const MAX_TASK_BYTES: usize = 0x0100_0000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub enum TaskOutput {
    Url(String),
    Fields(BTreeMap<String, String>),
    Unsupported(String),
}

impl From<Box<RawValue>> for TaskOutput {
    fn from(raw: Box<RawValue>) -> Self {
        if let Ok(url) = serde_json::from_str(raw.get()) {
            Self::Url(url)
        } else if let Ok(fields) = serde_json::from_str(raw.get()) {
            Self::Fields(fields)
        } else {
            Self::Unsupported(raw.get().to_owned())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub enum TaskOutputs {
    Members(Vec<TaskOutput>),
    NotAList(String),
}

impl From<Box<RawValue>> for TaskOutputs {
    fn from(raw: Box<RawValue>) -> Self {
        match serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) {
            Ok(members) => Self::Members(members.into_iter().map(TaskOutput::from).collect()),
            Err(_) => Self::NotAList(raw.get().to_owned()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunwayTask {
    pub id: Option<String>,
    pub status: Option<String>,
    pub failure: Option<String>,
    #[serde(rename = "failureCode")]
    pub failure_code: Option<String>,
    pub output: Option<TaskOutputs>,
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

    pub fn failure_error(&self, kind: TaskKind) -> RunwayTaskError {
        RunwayTaskError::Failed {
            kind,
            reason: self
                .failure
                .clone()
                .unwrap_or_else(|| "Unknown error".to_owned()),
            code: self
                .failure_code
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
        }
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
