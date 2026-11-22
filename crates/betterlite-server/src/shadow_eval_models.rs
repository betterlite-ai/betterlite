pub mod job;
pub mod leg;
pub mod results;

mod tests;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ShadowEvalStatus and ShadowEvalTargetType of the shadow eval response models
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowEvalStatus {
    Running,
    Completed,
    Stopped,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowEvalTargetType {
    Key,
    Team,
    User,
}

// a response the model refuses to build: pydantic rejects these inputs before status or router_name is read
#[derive(Debug, Eq, Error, PartialEq)]
pub enum ShadowEvalModelError {
    #[error("a shadow eval job has at least one target")]
    NoTargets,
    #[error("a shadow eval job has at least one router name")]
    NoRouterArms,
}
