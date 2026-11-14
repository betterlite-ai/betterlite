use std::sync::Arc;

use super::{CounterScript, ScriptCall, ScriptHandle, WindowStore};

#[must_use]
pub fn pipeline_scripts(
    store: &WindowStore,
    script: &Arc<dyn CounterScript>,
    calls: &[ScriptCall],
) -> Vec<Option<ScriptHandle>> {
    let Some(batch) = store
        .request_batch
        .as_ref()
        .and_then(|source| source.active_batch())
    else {
        return calls.iter().map(|_call| None).collect();
    };
    calls
        .iter()
        .map(|(keys, args)| Some(batch.script(Arc::clone(script), keys, args)))
        .collect()
}

mod tests;
