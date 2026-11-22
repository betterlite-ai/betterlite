use std::collections::BTreeSet;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_router::model_catalog_lookup::error::LookupError;

use crate::model_group_data::ports::ModelNameSource;
use crate::route_read_through::ModelReadThrough;

// A router stand-in: whether one is installed, the names it serves, and which a read-through brings live; it mutates itself as the reload rebinding the router would.
pub struct ScriptedRouter {
    state: Mutex<ScriptedRouterState>,
}

struct ScriptedRouterState {
    present: bool,
    names: BTreeSet<String>,
    read_through_returns: BTreeSet<String>,
    attempted: Vec<String>,
}

impl ScriptedRouter {
    pub fn new(names: Vec<String>, read_through_returns: Vec<String>) -> Self {
        Self::build(true, names, read_through_returns)
    }

    pub fn absent(names: Vec<String>, read_through_returns: Vec<String>) -> Self {
        Self::build(false, names, read_through_returns)
    }

    fn build(present: bool, names: Vec<String>, read_through_returns: Vec<String>) -> Self {
        Self {
            state: Mutex::new(ScriptedRouterState {
                present,
                names: names.into_iter().collect(),
                read_through_returns: read_through_returns.into_iter().collect(),
                attempted: Vec::new(),
            }),
        }
    }

    pub fn attempted(&self) -> Vec<String> {
        let state = self.state.lock().expect("scripted router");
        state.attempted.clone()
    }
}

impl ModelNameSource for ScriptedRouter {
    fn model_names(&self) -> Result<Option<Vec<String>>, LookupError> {
        let state = self.state.lock().expect("scripted router");
        Ok(state.present.then(|| state.names.iter().cloned().collect()))
    }
}

#[async_trait]
impl ModelReadThrough for ScriptedRouter {
    async fn attempt_model(&self, model: &str) -> bool {
        let mut state = self.state.lock().expect("scripted router");
        state.attempted.push(model.to_owned());
        if !state.read_through_returns.contains(model) {
            return false;
        }
        state.present = true;
        state.names.insert(model.to_owned());
        true
    }
}
