use std::error::Error;

use betterlite_policy::registry::RegistryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum Operation {
    #[strum(to_string = "adding policy to DB")]
    AddPolicy,
    #[strum(to_string = "updating policy in DB")]
    UpdatePolicy,
    #[strum(to_string = "deleting policy from DB")]
    DeletePolicy,
    #[strum(to_string = "getting policies from DB")]
    ListPolicies,
    #[strum(to_string = "syncing policies from DB")]
    SyncPolicies,
    #[strum(to_string = "resolving guardrails from DB")]
    ResolveGuardrails,
    #[strum(to_string = "adding attachment to DB")]
    AddAttachment,
    #[strum(to_string = "deleting attachment from DB")]
    DeleteAttachment,
    #[strum(to_string = "getting attachment from DB")]
    GetAttachment,
    #[strum(to_string = "getting attachments from DB")]
    ListAttachments,
    #[strum(to_string = "syncing attachments from DB")]
    SyncAttachments,
}

// LiteLLM wraps a failed inner call once per caller, so the message names every operation it crossed
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError<Failure>
where
    Failure: Error + Send + Sync + 'static,
{
    #[error("Error {}: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(": Error "), .1)]
    Database(Vec<Operation>, #[source] Failure),
    #[error("Error {}: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(": Error "), .1)]
    Json(Vec<Operation>, #[source] serde_json::Error),
    #[error("Error {}: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(": Error "), .1)]
    Policy(Vec<Operation>, #[source] RegistryError),
    #[error("Error {}: Policy with ID {} not found", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(": Error "), .1)]
    PolicyNotFound(Vec<Operation>, String),
    #[error("Error {}: Only draft versions can be updated. This policy has status '{}'.", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(": Error "), .1)]
    NotDraft(Vec<Operation>, String),
    #[error("Error {}: Attachment with ID {} not found", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(": Error "), .1)]
    AttachmentNotFound(Vec<Operation>, String),
}

impl<Failure> PersistenceError<Failure>
where
    Failure: Error + Send + Sync + 'static,
{
    #[must_use]
    pub fn logged(self) -> Self {
        tracing::error!("{self}");
        self
    }

    #[must_use]
    pub fn within(mut self, outer: Operation) -> Self {
        let (Self::Database(chain, _)
        | Self::Json(chain, _)
        | Self::Policy(chain, _)
        | Self::PolicyNotFound(chain, _)
        | Self::NotDraft(chain, _)
        | Self::AttachmentNotFound(chain, _)) = &mut self;
        chain.insert(0, outer);
        self
    }
}
