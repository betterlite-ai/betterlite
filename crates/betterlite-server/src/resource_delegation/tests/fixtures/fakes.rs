use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_auth::auth_model_patterns::access_groups::{
    AccessGroupError, AccessGroupLookup, AccessGroupResources,
};
use betterlite_mcp::access_group_servers::AccessGroupServerRows;

use super::strings;

#[derive(Debug, thiserror::Error)]
#[error("the fake rows port failed")]
pub(crate) struct PortFailure;

pub(crate) struct FakeRows {
    server_ids: Vec<String>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl FakeRows {
    pub(crate) fn answering(server_ids: &[&str]) -> Self {
        Self {
            server_ids: strings(server_ids),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn silent() -> Self {
        Self {
            server_ids: Vec::new(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl AccessGroupServerRows for FakeRows {
    type Error = PortFailure;

    fn server_ids_with_any_access_group(
        &self,
        access_groups: &[String],
        _use_writer: bool,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send {
        self.calls.lock().unwrap().push(access_groups.to_vec());
        ready(Ok(self.server_ids.clone()))
    }
}

#[derive(Debug)]
pub(crate) struct FakeGroups {
    groups: BTreeMap<String, AccessGroupResources>,
    calls: Mutex<Vec<String>>,
}

impl FakeGroups {
    pub(crate) fn answering(groups: &[(&str, &[&str])]) -> Self {
        let mut resources = BTreeMap::new();
        for (group_id, server_ids) in groups {
            resources.insert(
                String::from(*group_id),
                AccessGroupResources {
                    model_names: Vec::new(),
                    mcp_server_ids: strings(server_ids),
                    agent_ids: Vec::new(),
                },
            );
        }
        Self {
            groups: resources,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl AccessGroupLookup for FakeGroups {
    fn access_group(
        &self,
        access_group_id: &str,
    ) -> Result<AccessGroupResources, AccessGroupError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(String::from(access_group_id));
        self.groups
            .get(access_group_id)
            .cloned()
            .ok_or_else(|| AccessGroupError::Unavailable {
                access_group_id: String::from(access_group_id),
            })
    }
}
