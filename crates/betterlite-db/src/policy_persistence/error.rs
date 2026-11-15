use std::error::Error;
use std::fmt;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationChain(pub Vec<Operation>);

impl From<Operation> for OperationChain {
    fn from(operation: Operation) -> Self {
        Self(vec![operation])
    }
}

impl fmt::Display for OperationChain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        formatter.write_str(&names.join(": Error "))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PersistenceError<Failure>
where
    Failure: Error + Send + Sync + 'static,
{
    #[error("Error {0}: {1}")]
    Database(OperationChain, #[source] Failure),
    #[error("Error {0}: {1}")]
    Json(OperationChain, #[source] serde_json::Error),
    #[error("Error {0}: {1}")]
    Policy(OperationChain, #[source] RegistryError),
    #[error("Error {0}: Policy with ID {1} not found")]
    PolicyNotFound(OperationChain, String),
    #[error("Error {0}: Only draft versions can be updated. This policy has status '{1}'.")]
    NotDraft(OperationChain, String),
    #[error("Error {0}: Attachment with ID {1} not found")]
    AttachmentNotFound(OperationChain, String),
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
        chain.0.insert(0, outer);
        self
    }
}
