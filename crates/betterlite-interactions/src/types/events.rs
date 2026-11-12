use serde::{Deserialize, Serialize};

use super::content::Content;
use super::delta::ContentDeltaValue;
use super::interaction::Interaction;
use super::json::Object;
use super::status::InteractionStatus;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "event_type")]
pub enum InteractionSseEvent {
    #[serde(rename = "step.start")]
    StepStart(StepStart),
    #[serde(rename = "step.delta")]
    StepDelta(StepDelta),
    #[serde(rename = "step.stop")]
    StepStop(StepStop),
    #[serde(rename = "interaction.created")]
    InteractionCreated(InteractionCreated),
    #[serde(rename = "interaction.in_progress")]
    InteractionInProgress(InteractionInProgress),
    #[serde(rename = "interaction.completed")]
    InteractionCompleted(InteractionCompleted),
    #[serde(rename = "interaction.requires_action")]
    InteractionRequiresAction(InteractionRequiresAction),
    #[serde(rename = "interaction.start")]
    InteractionStart(InteractionEvent),
    #[serde(rename = "interaction.complete")]
    InteractionComplete(InteractionEvent),
    #[serde(rename = "interaction.status_update")]
    InteractionStatusUpdate(InteractionStatusUpdate),
    #[serde(rename = "content.start")]
    ContentStart(ContentStart),
    #[serde(rename = "content.delta")]
    ContentDelta(ContentDelta),
    #[serde(rename = "content.stop")]
    ContentStop(ContentStop),
    #[serde(rename = "error")]
    Error(ErrorEvent),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StepStart {
    pub index: Option<i64>,
    pub step: Option<Object>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StepDelta {
    pub index: Option<i64>,
    pub delta: Option<Object>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct StepStop {
    pub index: Option<i64>,
    pub status: Option<String>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionCreated {
    pub interaction: Option<Object>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionInProgress {
    pub interaction_id: Option<String>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionCompleted {
    pub interaction: Option<Object>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionRequiresAction {
    pub interaction_id: Option<String>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct InteractionEvent {
    pub interaction: Option<Interaction>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionStatusUpdate {
    pub interaction_id: Option<String>,
    pub status: Option<InteractionStatus>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContentStart {
    pub index: Option<i64>,
    pub content: Option<Content>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContentDelta {
    pub index: Option<i64>,
    pub delta: Option<ContentDeltaValue>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ContentStop {
    pub index: Option<i64>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ErrorEvent {
    pub error: Option<ErrorDetail>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ErrorDetail {
    pub code: Option<String>,
    pub message: Option<String>,
}
