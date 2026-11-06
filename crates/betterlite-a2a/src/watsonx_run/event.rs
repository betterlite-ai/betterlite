use serde::Deserialize;

use crate::watsonx_transform::extract::WxoRunResult;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct RunEvent {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, flatten)]
    pub run: WxoRunResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    Completed,
    Succeeded,
    Failed,
    Error,
    Cancelled,
}

impl RunState {
    #[must_use]
    pub fn parse(status: &str) -> Option<Self> {
        match status {
            "completed" => Some(Self::Completed),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "error" => Some(Self::Error),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}
