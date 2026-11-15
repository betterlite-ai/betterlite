mod agent;
mod authentication;
mod binding;
pub mod cache;
mod enrollment;
pub mod error;
pub mod records;
mod repository;
pub mod resolve;
mod retired;
mod subject;
mod tests;
mod unbound;

use std::sync::Arc;

use chrono::{DateTime, Utc};

use self::cache::ManagementObjectCache;
use self::records::{
    AgentRecords, IdentityRecords, RetiredAgentRecords, RetiredIdentityRecords,
    VerifiedSubjectRecords,
};

// LiteLLM's five typed Prisma repositories and its optional shared user cache, over one database client
#[derive(Clone)]
pub struct AgentIdentityStore {
    pub agents: Arc<dyn AgentRecords>,
    pub identities: Arc<dyn IdentityRecords>,
    pub humans: Arc<dyn VerifiedSubjectRecords>,
    pub retired: Option<Arc<dyn RetiredIdentityRecords>>,
    pub retired_agents: Option<Arc<dyn RetiredAgentRecords>>,
    pub cache: Option<Arc<dyn ManagementObjectCache>>,
    pub clock: fn() -> DateTime<Utc>,
}
