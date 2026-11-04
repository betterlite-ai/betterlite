use std::collections::BTreeMap;
use std::fmt::Debug;
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

#[must_use]
#[derive(Debug, Clone)]
pub struct CallContext<'context> {
    id: Uuid,
    deadline: Option<SystemTime>,
    parent_id: Option<Uuid>,
    internal: bool,
    privacy: bool,
    phase: Phase,
    billing_time: SystemTime,
    metadata: Option<&'context BTreeMap<String, Option<&'context dyn Debug>>>,
    http_client: Option<&'context dyn Debug>,
    azure_client: Option<&'context dyn Debug>,
    openai_client: Option<&'context dyn Debug>,
}

#[derive(Debug, Clone, Copy)]
pub struct CallContextView<'context> {
    pub id: Uuid,
    pub deadline: Option<SystemTime>,
    pub parent_id: Option<Uuid>,
    pub billing: BillingPolicy,
    pub privacy: bool,
    pub phase: Phase,
    pub billing_time: SystemTime,
    pub metadata: Option<&'context BTreeMap<String, Option<&'context dyn Debug>>>,
    pub http_client: Option<&'context dyn Debug>,
    pub azure_client: Option<&'context dyn Debug>,
    pub openai_client: Option<&'context dyn Debug>,
}

impl<'context> CallContext<'context> {
    #[must_use]
    pub const fn billing_policy(&self) -> BillingPolicy {
        if self.internal {
            BillingPolicy::Suppress
        } else {
            BillingPolicy::Include
        }
    }
    pub fn with_id(id: Uuid, now: SystemTime) -> Self {
        Self {
            id,
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
    pub const fn with_deadline(mut self, deadline: SystemTime) -> Self {
        self.deadline = Some(deadline);
        self
    }
    pub const fn with_privacy(mut self, privacy: bool) -> Self {
        self.privacy = privacy;
        self
    }
    pub fn with_metadata(
        mut self,
        metadata: &'context BTreeMap<String, Option<&'context dyn Debug>>,
    ) -> Self {
        self.metadata = Some(metadata);
        self
    }
    pub fn with_http_client(mut self, client: &'context dyn Debug) -> Self {
        self.http_client = Some(client);
        self
    }
    pub fn with_azure_client(mut self, client: &'context dyn Debug) -> Self {
        self.azure_client = Some(client);
        self
    }
    pub fn with_openai_client(mut self, client: &'context dyn Debug) -> Self {
        self.openai_client = Some(client);
        self
    }
    pub fn child(&self, id: Uuid) -> Self {
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
    #[must_use]
    pub fn view(&self) -> CallContextView<'context> {
        CallContextView {
            id: self.id,
            deadline: self.deadline,
            parent_id: self.parent_id,
            billing: self.billing_policy(),
            privacy: self.privacy,
            phase: self.phase,
            billing_time: self.billing_time,
            metadata: self.metadata,
            http_client: self.http_client,
            azure_client: self.azure_client,
            openai_client: self.openai_client,
        }
    }
}
