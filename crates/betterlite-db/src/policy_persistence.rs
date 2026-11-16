pub mod attachment_creation;
pub mod attachment_deletion;
pub mod attachment_list;
pub mod attachment_retrieval;
pub mod attachment_row;
pub mod attachment_sync;
pub mod error;
mod existing_policy;
pub mod policy_creation;
pub mod policy_deletion;
pub mod policy_list;
pub mod policy_resolve;
pub mod policy_sync;
pub mod policy_update;
mod request_pipeline;
mod stored_policies;

mod tests;

use std::sync::Mutex;

use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::registry::PolicyRegistry;
use betterlite_policy::row_adapter::{PolicyRow, PolicyTable, PolicyTableSource};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use self::attachment_row::{AttachmentRow, AttachmentTableSource};
use self::error::PersistenceError;

pub type PolicyFailure<Client> =
    <<Client as PolicyTableSource>::Table as PolicyTable<PolicyRow>>::Failure;

pub type AttachmentFailure<Client> =
    <<Client as AttachmentTableSource>::Table as PolicyTable<AttachmentRow>>::Failure;

pub type PolicyError<Client> = PersistenceError<PolicyFailure<Client>>;

pub type AttachmentError<Client> = PersistenceError<AttachmentFailure<Client>>;

pub struct PolicyStore<'store, Client> {
    pub client: &'store Client,
    pub registry: &'store Mutex<PolicyRegistry>,
    pub now: fn() -> DateTime<Utc>,
}

pub struct AttachmentStore<'store, Client> {
    pub client: &'store Client,
    pub registry: &'store Mutex<AttachmentRegistry>,
    pub now: fn() -> DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deletion {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}
