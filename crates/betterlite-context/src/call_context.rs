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

pub struct CallInputs<Clock, Id> {
    pub now: Clock,
    pub id: Id,
    pub deadline: Option<SystemTime>,
    pub privacy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallState {
    pub id: Uuid,
    pub deadline: Option<SystemTime>,
    pub parent_id: Option<Uuid>,
    pub billing: BillingPolicy,
    pub privacy: bool,
    pub phase: Phase,
    pub billing_time: SystemTime,
}

#[must_use]
#[derive(Debug)]
pub struct CallContext<'context, Metadata = (), Client = ()> {
    state: CallState,
    metadata: Option<&'context Metadata>,
    client: Option<&'context Client>,
}

#[derive(Debug)]
pub struct CallContextView<'context, Metadata, Client> {
    pub state: &'context CallState,
    pub metadata: Option<&'context Metadata>,
    pub client: Option<&'context Client>,
}

impl CallContext<'static> {
    pub fn start<Clock, Id>(inputs: CallInputs<Clock, Id>) -> Self
    where
        Clock: FnOnce() -> SystemTime,
        Id: FnOnce() -> Uuid,
    {
        let id = (inputs.id)();
        let billing_time = (inputs.now)();
        Self {
            state: CallState {
                id,
                deadline: inputs.deadline,
                parent_id: None,
                billing: BillingPolicy::Include,
                privacy: inputs.privacy,
                phase: Phase::Request,
                billing_time,
            },
            metadata: None,
            client: None,
        }
    }
}

impl<Metadata, Client> CallContext<'_, Metadata, Client> {
    pub const fn with_dispatch<'dispatch, NewMetadata, NewClient>(
        self,
        metadata: Option<&'dispatch NewMetadata>,
        client: Option<&'dispatch NewClient>,
    ) -> CallContext<'dispatch, NewMetadata, NewClient> {
        CallContext {
            state: self.state,
            metadata,
            client,
        }
    }

    pub const fn child(&self, id: Uuid) -> Self {
        let mut state = self.state;
        state.id = id;
        state.parent_id = Some(self.state.id);
        state.billing = BillingPolicy::Suppress;
        Self {
            state,
            metadata: self.metadata,
            client: self.client,
        }
    }

    pub const fn post_response(&self) -> Self {
        let mut state = self.state;
        state.phase = Phase::PostResponse;
        Self {
            state,
            metadata: self.metadata,
            client: self.client,
        }
    }

    #[must_use]
    pub const fn view(&self) -> CallContextView<'_, Metadata, Client> {
        CallContextView {
            state: &self.state,
            metadata: self.metadata,
            client: self.client,
        }
    }
}
