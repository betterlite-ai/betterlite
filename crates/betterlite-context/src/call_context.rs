use std::collections::BTreeMap;
use std::time::SystemTime;

use uuid::Uuid;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Request,
    PostResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingPolicy {
    Include,
    Suppress,
}

#[derive(Debug, Clone)]
pub struct CallContext<'a> {
    pub id: Uuid,
    pub deadline: Option<SystemTime>,
    pub parent_id: Option<Uuid>,
    internal: bool,
    pub privacy: bool,
    pub phase: Phase,
    pub billing_time: SystemTime,
    pub metadata: Option<&'a BTreeMap<String, serde_json::Value>>,
    pub http_client: Option<&'a serde_json::Value>,
    pub azure_client: Option<&'a serde_json::Value>,
    pub openai_client: Option<&'a serde_json::Value>,
}

impl<'a> CallContext<'a> {
    pub fn billing_policy(&self) -> BillingPolicy {
        match self.internal {
            true => BillingPolicy::Suppress,
            false => BillingPolicy::Include,
        }
    }
    pub fn new(now: SystemTime) -> Self {
        Self {
            id: Uuid::new_v4(),
            deadline: None,
            parent_id: None,
            internal: false,
            privacy: false,
            phase: Phase::Request,
            billing_time: now,
            metadata: None,
            http_client: None,
            azure_client: None,
            openai_client: None,
        }
    }
    pub fn with_id(id: Uuid, now: SystemTime) -> Self {
        Self {
            id,
            ..Self::new(now)
        }
    }
    pub fn with_deadline(mut self, deadline: SystemTime) -> Self {
        self.deadline = Some(deadline);
        self
    }
    pub fn with_privacy(mut self, privacy: bool) -> Self {
        self.privacy = privacy;
        self
    }
    pub fn with_metadata(mut self, metadata: &'a BTreeMap<String, serde_json::Value>) -> Self {
        self.metadata = Some(metadata);
        self
    }
    pub fn with_http_client(mut self, client: &'a serde_json::Value) -> Self {
        self.http_client = Some(client);
        self
    }
    pub fn with_azure_client(mut self, client: &'a serde_json::Value) -> Self {
        self.azure_client = Some(client);
        self
    }
    pub fn with_openai_client(mut self, client: &'a serde_json::Value) -> Self {
        self.openai_client = Some(client);
        self
    }
    pub fn child(&self, id: Uuid) -> CallContext<'a> {
        CallContext {
            id,
            deadline: self.deadline,
            parent_id: Some(self.id),
            internal: true,
            privacy: self.privacy,
            phase: self.phase,
            billing_time: self.billing_time,
            metadata: self.metadata,
            http_client: self.http_client,
            azure_client: self.azure_client,
            openai_client: self.openai_client,
        }
    }
    pub fn post_response(&self) -> Self {
        let mut child = self.clone();
        child.phase = Phase::PostResponse;
        child
    }
}
