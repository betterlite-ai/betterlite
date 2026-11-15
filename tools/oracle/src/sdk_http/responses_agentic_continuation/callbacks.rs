use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::{RawValue, to_raw_value};

use super::{CallbackScript, TRACED_KEYS, TRACED_PREFIX};
use crate::sdk_http::result::AgenticTrace;

fn traced_keys(kwargs: &JsonMap) -> Vec<String> {
    let mut keys: Vec<String> = kwargs
        .keys()
        .filter(|key| key.starts_with(TRACED_PREFIX) || TRACED_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    keys.sort_unstable();
    keys
}

pub(super) struct ScriptedCallback {
    pub(super) script: CallbackScript,
    pub(super) trace: Arc<Mutex<Vec<AgenticTrace>>>,
}

impl ScriptedCallback {
    fn record(&self, hook: &str, kwargs: &JsonMap) {
        let mut trace = match self.trace.lock() {
            Ok(trace) => trace,
            Err(poisoned) => poisoned.into_inner(),
        };
        trace.push(AgenticTrace {
            callback: self.script.name.clone(),
            hook: hook.to_owned(),
            kwargs: traced_keys(kwargs),
        });
    }

    fn gate_tools(&self, tools: &IndexMap<String, Json>) -> Result<JsonMap, AgenticHookError> {
        let mut map = JsonMap::with_capacity(tools.len());
        for (key, value) in tools {
            let raw = to_raw_value(value).map_err(|source| AgenticHookError::Failed {
                callback: self.script.name.clone(),
                message: format!("the case gate tools do not encode: {source}"),
            })?;
            map.insert(key.clone(), raw);
        }
        Ok(map)
    }

    fn case_plan(
        &self,
        plan: &IndexMap<String, Json>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        let text = serde_json::to_string(plan).map_err(|source| AgenticHookError::Failed {
            callback: self.script.name.clone(),
            message: format!("the case plan does not encode: {source}"),
        })?;
        serde_json::from_str::<AgenticLoopPlan>(&text).map_err(|source| AgenticHookError::Failed {
            callback: self.script.name.clone(),
            message: format!("the case plan does not decode: {source}"),
        })
    }
}

#[async_trait]
impl AgenticCallback for ScriptedCallback {
    fn callback_name(&self) -> &str {
        &self.script.name
    }

    fn overrides(&self) -> AgenticOverrides {
        let plans = self.script.plan.is_some() || self.script.plan_error.is_some();
        AgenticOverrides {
            should_run: true,
            run_loop: self.script.legacy,
            build_plan: plans,
            post_response: plans,
            cleanup: plans,
        }
    }

    async fn should_run_agentic_loop(
        &self,
        call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        self.record("gate", call.kwargs);
        if let Some(message) = &self.script.gate_error {
            return Err(AgenticHookError::Failed {
                callback: self.script.name.clone(),
                message: message.clone(),
            });
        }
        let Some(gate) = &self.script.gate else {
            return Ok(AgenticGate::default());
        };
        Ok(AgenticGate {
            run: gate.run,
            tools: self.gate_tools(&gate.tools)?,
        })
    }

    async fn build_agentic_loop_plan(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        self.record("plan", call.kwargs);
        if let Some(message) = &self.script.plan_error {
            return Err(AgenticHookError::Failed {
                callback: self.script.name.clone(),
                message: message.clone(),
            });
        }
        let Some(plan) = &self.script.plan else {
            return Ok(AgenticLoopPlan::default());
        };
        self.case_plan(plan)
    }

    async fn run_agentic_loop(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        if self.script.legacy {
            self.record("legacy", call.kwargs);
        }
        Ok(None)
    }

    async fn post_agentic_loop_response(
        &self,
        response: Box<RawValue>,
        _plan: &AgenticLoopPlan,
        kwargs: &JsonMap,
    ) -> Result<Box<RawValue>, AgenticHookError> {
        self.record("post", kwargs);
        Ok(response)
    }

    async fn agentic_loop_cleanup(
        &self,
        _plan: &AgenticLoopPlan,
        kwargs: &JsonMap,
    ) -> Result<(), AgenticHookError> {
        self.record("cleanup", kwargs);
        Ok(())
    }
}
