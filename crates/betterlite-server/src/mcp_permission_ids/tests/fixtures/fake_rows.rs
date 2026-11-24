use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_mcp::access_group_servers::AccessGroupServerRows;

#[derive(Debug, thiserror::Error)]
#[error("the fake rows port failed")]
pub(crate) struct PortFailure;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PortCall {
    pub(crate) access_groups: Vec<String>,
    pub(crate) use_writer: bool,
}

pub(crate) struct FakeRows {
    server_ids: Vec<String>,
    failing: bool,
    calls: Mutex<Vec<PortCall>>,
}

impl FakeRows {
    pub(crate) fn answering(server_ids: &[&str]) -> Self {
        Self {
            server_ids: server_ids
                .iter()
                .map(|server_id| String::from(*server_id))
                .collect(),
            failing: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            server_ids: Vec::new(),
            failing: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<PortCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl AccessGroupServerRows for FakeRows {
    type Error = PortFailure;

    fn server_ids_with_any_access_group(
        &self,
        access_groups: &[String],
        use_writer: bool,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send {
        self.calls.lock().unwrap().push(PortCall {
            access_groups: access_groups.to_vec(),
            use_writer,
        });
        if self.failing {
            return ready(Err(PortFailure));
        }
        ready(Ok(self.server_ids.clone()))
    }
}
