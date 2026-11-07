use crate::client::convert::messages::to_compat_message;
use crate::client::convert::parts::to_compat_part;
use crate::client::records::compat::{Artifact, Task, TaskKind, TaskState, TaskStatus};
use crate::client::records::core::{CoreArtifact, CoreTask, CoreTaskState, CoreTaskStatus};

#[rustfmt::skip]
pub fn to_compat_task(task: &CoreTask) -> Task {
    let status = compat_status_of(task.status.as_ref());
    let history = converted_list(&task.history, to_compat_message);
    let artifacts = converted_list(&task.artifacts, to_compat_artifact);
    Task { kind: TaskKind::Task, status, history, artifacts, metadata: task.metadata.clone(), id: task.id.clone(), context_id: task.context_id.clone().unwrap_or_default() }
}

#[must_use]
#[rustfmt::skip]
pub fn compat_status_of(status: Option<&CoreTaskStatus>) -> TaskStatus {
    let Some(present) = status else {
        return TaskStatus { state: TaskState::Unknown, message: None, timestamp: None };
    };
    to_compat_status(present)
}

#[must_use]
pub const fn is_final(status: &TaskStatus) -> bool {
    matches!(
        status.state,
        TaskState::Completed | TaskState::Canceled | TaskState::Failed | TaskState::Rejected
    )
}

#[rustfmt::skip]
pub fn to_compat_status(status: &CoreTaskStatus) -> TaskStatus {
    let message = status.message.as_ref().map(to_compat_message);
    let state = compat_state(status.state);
    TaskStatus { state, message, timestamp: status.timestamp.clone().filter(|value| !value.is_empty()) }
}

#[rustfmt::skip]
const fn compat_state(state: CoreTaskState) -> TaskState {
    match state {
        CoreTaskState::Unspecified => TaskState::Unknown, CoreTaskState::Submitted => TaskState::Submitted, CoreTaskState::Working => TaskState::Working, CoreTaskState::Completed => TaskState::Completed, CoreTaskState::Failed => TaskState::Failed, CoreTaskState::Canceled => TaskState::Canceled, CoreTaskState::InputRequired => TaskState::InputRequired, CoreTaskState::Rejected => TaskState::Rejected, CoreTaskState::AuthRequired => TaskState::AuthRequired,
    }
}

#[rustfmt::skip]
pub fn to_compat_artifact(artifact: &CoreArtifact) -> Artifact {
    let parts = artifact.parts.iter().map(to_compat_part).collect();
    let extensions = (!artifact.extensions.is_empty()).then(|| artifact.extensions.clone());
    Artifact { parts, extensions, metadata: artifact.metadata.clone(), artifact_id: artifact.artifact_id.clone(), name: artifact.name.clone().filter(|value| !value.is_empty()), description: artifact.description.clone().filter(|value| !value.is_empty()) }
}

#[rustfmt::skip]
fn converted_list<T, U, F>(items: &[T], convert: F) -> Option<Vec<U>> where F: Fn(&T) -> U {
    if items.is_empty() { return None; }
    Some(items.iter().map(convert).collect())
}
