use crate::client::convert::ConvertError;
use crate::client::convert::messages::to_compat_message;
use crate::client::convert::parts::to_compat_part;
use crate::client::records::compat::{Artifact, Task, TaskState, TaskStatus};
use crate::client::records::core::{CoreArtifact, CoreTask, CoreTaskState, CoreTaskStatus};

pub fn to_compat_task(task: &CoreTask) -> Result<Task, ConvertError> {
    let status = compat_status_of(task.status.as_ref())?;
    let history = convert_list(&task.history, to_compat_message)?;
    let artifacts = convert_list(&task.artifacts, to_compat_artifact)?;
    Ok(Task {
        status,
        history,
        artifacts,
        metadata: task.metadata.clone(),
        id: task.id.clone(),
        context_id: task.context_id.clone().unwrap_or_default(),
    })
}

pub fn compat_status_of(status: Option<&CoreTaskStatus>) -> Result<TaskStatus, ConvertError> {
    let Some(present) = status else {
        return Ok(unknown_status());
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

pub fn to_compat_status(status: &CoreTaskStatus) -> Result<TaskStatus, ConvertError> {
    Ok(TaskStatus {
        state: compat_state(status.state),
        message: status.message.as_ref().map(to_compat_message).transpose()?,
        timestamp: status.timestamp.clone().filter(|value| !value.is_empty()),
    })
}

const fn unknown_status() -> TaskStatus {
    TaskStatus {
        state: TaskState::Unknown,
        message: None,
        timestamp: None,
    }
}

const fn compat_state(state: CoreTaskState) -> TaskState {
    match state {
        CoreTaskState::Unspecified => TaskState::Unknown,
        CoreTaskState::Submitted => TaskState::Submitted,
        CoreTaskState::Working => TaskState::Working,
        CoreTaskState::Completed => TaskState::Completed,
        CoreTaskState::Failed => TaskState::Failed,
        CoreTaskState::Canceled => TaskState::Canceled,
        CoreTaskState::InputRequired => TaskState::InputRequired,
        CoreTaskState::Rejected => TaskState::Rejected,
        CoreTaskState::AuthRequired => TaskState::AuthRequired,
    }
}

pub fn to_compat_artifact(artifact: &CoreArtifact) -> Result<Artifact, ConvertError> {
    Ok(Artifact {
        parts: artifact.parts.iter().map(to_compat_part).collect(),
        metadata: artifact.metadata.clone(),
        extensions: (!artifact.extensions.is_empty()).then(|| artifact.extensions.clone()),
        artifact_id: artifact.artifact_id.clone(),
        name: artifact.name.clone().filter(|value| !value.is_empty()),
        description: artifact
            .description
            .clone()
            .filter(|value| !value.is_empty()),
    })
}

fn convert_list<T, U, F>(items: &[T], convert: F) -> Result<Option<Vec<U>>, ConvertError>
where
    F: Fn(&T) -> Result<U, ConvertError>,
{
    if items.is_empty() {
        return Ok(None);
    }
    let converted = items.iter().map(convert).collect::<Result<Vec<_>, _>>()?;
    Ok(Some(converted))
}
