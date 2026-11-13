use std::sync::PoisonError;

use betterlite_models::available_models::cache::{AvailableModelsCache, CacheError};
use betterlite_models::available_models::environment::EnvironmentSnapshot;
use serde_json::{Value, json};

use super::clock;
use super::input::{CacheInput, CacheOp, CacheStep};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn run(fields: &CacheInput) -> HelperResult {
    let (state, switchable) = clock::switchable();
    let mut cache = AvailableModelsCache::with_clock(switchable);
    let mut steps = Vec::new();
    for step in &fields.steps {
        {
            let mut locked = state.lock().unwrap_or_else(PoisonError::into_inner);
            locked.now = step.now;
            locked.fails = step.clock_fails;
        };
        let environment = EnvironmentSnapshot::from(&step.environment);
        steps.push(step_output(&mut cache, step, &environment)?);
    }
    Ok(json!({ "steps": steps }))
}

fn step_output(
    cache: &mut AvailableModelsCache,
    step: &CacheStep,
    environment: &EnvironmentSnapshot,
) -> Result<Value, Box<HelperError>> {
    match step.op {
        CacheOp::Config => Ok(json!({
            "config": {
                "capacity": cache.limits.capacity,
                "default_ttl": cache.limits.default_ttl,
            }
        })),
        CacheOp::Get => Ok(read_step(cache, step, environment, false)),
        CacheOp::GetMutate => Ok(read_step(cache, step, environment, true)),
        CacheOp::Set | CacheOp::SetMutate => write_step(cache, step),
    }
}

fn read_step(
    cache: &mut AvailableModelsCache,
    step: &CacheStep,
    environment: &EnvironmentSnapshot,
    mutate: bool,
) -> Value {
    let result = cache.get(step.provider, step.parameters.as_ref(), environment);
    report(result, step.op, |value| {
        let recorded = value.clone();
        if mutate {
            let mut mutated = value;
            if let Some(models) = &mut mutated {
                models.push("injected".to_owned());
            }
        }
        json!({"get": recorded})
    })
}

fn write_step(
    cache: &mut AvailableModelsCache,
    step: &CacheStep,
) -> Result<Value, Box<HelperError>> {
    let Some(provider) = step.provider else {
        return Err(invalid("a set step needs a provider".to_owned()));
    };
    let mut stored = step.models.clone();
    let result = cache.set(provider, step.parameters.as_ref(), &stored);
    Ok(report(result, step.op, |()| {
        if matches!(step.op, CacheOp::SetMutate) {
            stored.push("injected".to_owned());
        }
        json!({"set": null})
    }))
}

fn report<T, F>(result: Result<T, CacheError>, op: CacheOp, project: F) -> Value
where
    F: FnOnce(T) -> Value,
{
    match result {
        Ok(value) => project(value),
        Err(_error) => json!({"failed": {"op": op}}),
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
