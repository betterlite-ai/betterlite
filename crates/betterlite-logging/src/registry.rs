pub mod catalog;
pub mod error;
pub mod identity;
pub mod phases;
pub mod sink;
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;

use error::RegistryError;
use identity::CallbackIdentity;
use phases::{CallbackList, CallbackPhase, CallbackSelection};
use sink::{CallbackEvent, CallbackSink};

use crate::callback_config::{CallbackVars, FeatureFlags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    Registered,
    AlreadyRegistered,
    SkippedCapacity,
}

struct Entry {
    identity: CallbackIdentity,
    sink: CallbackSink,
    lists: BTreeSet<CallbackList>,
    phases: BTreeSet<CallbackPhase>,
}

impl Entry {
    fn rebuild_phases(&mut self) {
        self.phases = self
            .lists
            .iter()
            .flat_map(|list| list.phases())
            .copied()
            .collect();
    }
}

pub struct CallbackRegistry {
    capacity: usize,
    entries: Vec<Entry>,
    lists: BTreeMap<CallbackList, Vec<CallbackIdentity>>,
    configured_vars: Vec<CallbackVars>,
}

impl CallbackRegistry {
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Vec::new(),
            lists: BTreeMap::new(),
            configured_vars: Vec::new(),
        }
    }

    // LiteLLM checks a list's capacity before it looks for the duplicate, so a full list skips
    #[must_use]
    pub fn register(
        &mut self,
        identity: CallbackIdentity,
        sink: CallbackSink,
        list: CallbackList,
    ) -> Registration {
        let count = self.lists.get(&list).map_or(0, Vec::len);
        if count >= self.capacity {
            tracing::warn!(
                callback = %identity.class_name,
                list = ?list,
                capacity = self.capacity,
                "callback skipped: the list is at capacity"
            );
            return Registration::SkippedCapacity;
        }
        let key = identity.key();
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.identity.key() == key)
        {
            if !entry.lists.insert(list) {
                tracing::debug!(
                    callback = %entry.identity.class_name,
                    list = ?list,
                    "callback already registered in this list; keeping the first"
                );
                return Registration::AlreadyRegistered;
            }
            entry.rebuild_phases();
        } else {
            self.entries.push(Entry {
                identity: identity.clone(),
                sink,
                lists: BTreeSet::from([list]),
                phases: list.phases().iter().copied().collect(),
            });
        }
        self.lists.entry(list).or_default().push(identity);
        Registration::Registered
    }

    pub fn register_configured(
        &mut self,
        name: &str,
        vars: &CallbackVars,
        list: CallbackList,
        flags: FeatureFlags,
    ) -> Result<Registration, RegistryError> {
        catalog::validate_callback_vars(Some(name), vars, &self.configured_vars, flags)?;
        let sink = catalog::adapter_for(name)?;
        let identity = CallbackIdentity {
            class_name: name.to_owned(),
            attributes: IndexMap::new(),
        };
        self.configured_vars.push(vars.clone());
        Ok(self.register(identity, sink, list))
    }

    pub fn remove_from_list(&mut self, identity: &CallbackIdentity, list: CallbackList) -> bool {
        let key = identity.key();
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.identity.key() == key)
        else {
            return false;
        };
        if !entry.lists.remove(&list) {
            return false;
        }
        entry.rebuild_phases();
        let empty = entry.lists.is_empty();
        if empty {
            self.entries.retain(|stored| stored.identity.key() != key);
        }
        if let Some(order) = self.lists.get_mut(&list) {
            order.retain(|stored| stored.key() != key);
        }
        true
    }

    pub fn remove_class_from_list(&mut self, class_name: &str, list: CallbackList) -> usize {
        let Some(order) = self.lists.get_mut(&list) else {
            return 0;
        };
        let before = order.len();
        order.retain(|identity| identity.class_name != class_name);
        let removed = before.saturating_sub(order.len());
        if removed == 0 {
            return 0;
        }
        for entry in &mut self.entries {
            if entry.identity.class_name == class_name {
                entry.lists.remove(&list);
                entry.rebuild_phases();
            }
        }
        self.entries.retain(|entry| !entry.lists.is_empty());
        removed
    }

    #[must_use]
    pub fn select(&self) -> CallbackSelection {
        let mut selection = CallbackSelection {
            success: Vec::new(),
            failure: Vec::new(),
            success_and_failure: Vec::new(),
        };
        for entry in &self.entries {
            let in_success = entry.phases.contains(&CallbackPhase::Success);
            let in_failure = entry.phases.contains(&CallbackPhase::Failure);
            let name = entry.identity.class_name.clone();
            match (in_success, in_failure) {
                (true, true) => selection.success_and_failure.push(name),
                (true, false) => selection.success.push(name),
                (false, true) => selection.failure.push(name),
                (false, false) => {}
            }
        }
        selection.success = selection
            .success
            .into_iter()
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        selection.failure = selection
            .failure
            .into_iter()
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        selection.success_and_failure = selection
            .success_and_failure
            .into_iter()
            .collect::<BTreeSet<String>>()
            .into_iter()
            .collect();
        selection
    }

    pub fn deliver(&self, event: &CallbackEvent) {
        for entry in &self.entries {
            if !entry.phases.contains(&event.phase) {
                continue;
            }
            if let Err(error) = entry.sink.deliver(event) {
                tracing::warn!(
                    callback = %entry.identity.class_name,
                    %error,
                    "callback delivery failed"
                );
            }
        }
    }

    #[must_use]
    pub fn list_identities(&self, list: CallbackList) -> &[CallbackIdentity] {
        let Some(order) = self.lists.get(&list) else {
            return &[];
        };
        order
    }

    pub fn phase_identities(
        &self,
        phase: CallbackPhase,
    ) -> impl Iterator<Item = &CallbackIdentity> {
        self.entries
            .iter()
            .filter(move |entry| entry.phases.contains(&phase))
            .map(|entry| &entry.identity)
    }
}
