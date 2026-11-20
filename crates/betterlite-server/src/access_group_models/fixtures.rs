#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;

use betterlite_db::postgres_store::errors::DbError;

use super::{
    DeploymentCountRow, ModelGrantDatabase, ModelNameRewrite, ModelSyncError, TouchedGroupRow,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GrantCall {
    Count { model_name: String },
    Rewrite(ModelNameRewrite),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptedFailure {
    Count,
    Rewrite,
}

// Stands in for the three array statements over a table whose arrays keep duplicates and order.
pub(crate) struct ScriptedModelGrants {
    pub(crate) groups: Mutex<BTreeMap<String, Vec<String>>>,
    pub(crate) calls: Mutex<Vec<GrantCall>>,
    pub(crate) deployment_counts: Mutex<Vec<DeploymentCountRow>>,
    pub(crate) failure: Option<ScriptedFailure>,
    pub(crate) reverse_rows: bool,
}

impl ScriptedModelGrants {
    pub(crate) const fn new(groups: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            groups: Mutex::new(groups),
            calls: Mutex::new(Vec::new()),
            deployment_counts: Mutex::new(Vec::new()),
            failure: None,
            reverse_rows: false,
        }
    }

    fn apply(&self, rewrite: &ModelNameRewrite) -> Vec<TouchedGroupRow> {
        let mut groups = self.groups.lock().expect("groups");
        let mut touched = Vec::new();
        for (access_group_id, model_names) in groups.iter_mut() {
            let applied = match rewrite {
                ModelNameRewrite::Append { old_name, new_name } => {
                    append_name(model_names, old_name, new_name)
                }
                ModelNameRewrite::Replace { old_name, new_name } => {
                    replace_name(model_names, old_name, new_name)
                }
                ModelNameRewrite::Remove { model_name } => remove_name(model_names, model_name),
            };
            if let Some(names) = applied {
                *model_names = names;
                touched.push(TouchedGroupRow {
                    access_group_id: access_group_id.clone(),
                });
            }
        }
        drop(groups);
        if self.reverse_rows {
            touched.reverse();
        }
        touched
    }
}

fn append_name(model_names: &[String], old_name: &str, new_name: &str) -> Option<Vec<String>> {
    let holds_old = model_names.iter().any(|name| name == old_name);
    let holds_new = model_names.iter().any(|name| name == new_name);
    if !holds_old || holds_new {
        return None;
    }
    let mut names = model_names.to_vec();
    names.push(new_name.to_owned());
    Some(names)
}

fn replace_name(model_names: &[String], old_name: &str, new_name: &str) -> Option<Vec<String>> {
    if !model_names.iter().any(|name| name == old_name) {
        return None;
    }
    let names = model_names
        .iter()
        .filter(|name| name.as_str() != new_name)
        .map(|name| {
            if name == old_name {
                new_name.to_owned()
            } else {
                name.clone()
            }
        })
        .collect();
    Some(names)
}

fn remove_name(model_names: &[String], model_name: &str) -> Option<Vec<String>> {
    let (kept, removed): (Vec<String>, Vec<String>) = model_names
        .iter()
        .cloned()
        .partition(|name| name.as_str() != model_name);
    (!removed.is_empty()).then_some(kept)
}

impl ModelGrantDatabase for ScriptedModelGrants {
    fn backing_deployments(
        &self,
        model_name: &str,
    ) -> impl Future<Output = Result<Vec<DeploymentCountRow>, ModelSyncError>> + Send {
        let counts = self.deployment_counts.lock().expect("counts").clone();
        self.calls.lock().expect("calls").push(GrantCall::Count {
            model_name: model_name.to_owned(),
        });
        let failed = self.failure == Some(ScriptedFailure::Count);
        async move {
            if failed {
                return Err(ModelSyncError::Database(DbError::NoDbConnected));
            }
            Ok(counts)
        }
    }

    fn rewrite_model_names(
        &self,
        rewrite: &ModelNameRewrite,
    ) -> impl Future<Output = Result<Vec<TouchedGroupRow>, ModelSyncError>> + Send {
        let outcome = match self.failure {
            Some(ScriptedFailure::Rewrite) => Err(ModelSyncError::Database(DbError::NoDbConnected)),
            None | Some(ScriptedFailure::Count) => Ok(self.apply(rewrite)),
        };
        self.calls
            .lock()
            .expect("calls")
            .push(GrantCall::Rewrite(rewrite.clone()));
        async move { outcome }
    }
}
