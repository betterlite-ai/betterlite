use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

// one call the check made, with every field present so both helpers print the same shape
#[derive(Clone, Debug, Serialize)]
pub(super) struct CallRecord {
    pub call: &'static str,
    pub object_type: Option<&'static str>,
    pub model: Option<String>,
    pub models: Vec<String>,
    pub team_model_aliases: BTreeMap<String, String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub user_id_upsert: Option<bool>,
    pub project_id: Option<String>,
}

impl CallRecord {
    pub(super) fn new(call: &'static str) -> Self {
        Self {
            call,
            object_type: None,
            model: None,
            models: Vec::new(),
            team_model_aliases: BTreeMap::new(),
            team_id: None,
            user_id: None,
            user_id_upsert: None,
            project_id: None,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Journal(Arc<Mutex<Vec<CallRecord>>>);

impl Journal {
    pub(super) fn record(&self, record: CallRecord) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }

    pub(super) fn records(&self) -> Vec<CallRecord> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
