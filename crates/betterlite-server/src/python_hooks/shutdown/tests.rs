#![cfg(test)]

use std::time::{Duration, Instant};

use betterlite_pyhost::call_error::CallError;
use betterlite_pyhost::json_values::JsonObject;
use pyo3::types::{PyAnyMethods as _, PyDict, PyDictMethods as _};
use tokio::task::yield_now;

use crate::python_hooks::HooksRuntime;
use crate::python_hooks::slots::{HookSlot, LoadedHook};
use crate::python_hooks::test_support;

const GATED: &str = "import threading\n\n_gate = threading.Event()\n\n\
def blocked():\n    _gate.wait()\n    return \"done\"\n\n\
def release():\n    _gate.set()\n";

const WAIT_LIMIT: Duration = Duration::from_secs(10);

fn release(runtime: &HooksRuntime, hook: &LoadedHook) {
    // the loader execs the file without registering it, so release reaches the same module
    runtime.host.attach(|py| {
        let loaded = hook.object.loaded.object.bind(py);
        let globals = loaded.getattr("__globals__").unwrap();
        let entries = globals.cast_into::<PyDict>().unwrap();
        let release = entries.get_item("release").unwrap().unwrap();
        release.call0().unwrap();
    });
}

async fn wait_for_idle(runtime: &HooksRuntime) {
    let clock = Instant::now();
    while runtime.tracker.stats().running != 0 {
        assert!(
            clock.elapsed() < WAIT_LIMIT,
            "the abandoned job never finished"
        );
        yield_now().await;
    }
}

#[tokio::test]
async fn shutting_down_without_loading_a_hook_does_nothing() {
    let service = test_support::service(None);
    service.shutdown().await;
    assert!(service.inner.get().is_none());
}

#[tokio::test]
async fn shutdown_reports_the_jobs_still_running_after_the_grace() {
    let workspace = test_support::workspace();
    let module = workspace.module("b165_shutdown_job", GATED);
    let service = test_support::service(Some(workspace.config_path.clone()));
    let hook = service
        .load_hook(HookSlot::CustomAuth, &format!("{module}.blocked"))
        .await
        .unwrap();
    let error = hook
        .call_within(Duration::from_millis(500), Vec::new(), JsonObject::new())
        .await
        .unwrap_err();
    assert!(matches!(error, CallError::Work(_)), "got: {error:?}");
    let runtime = service.inner.get().unwrap();
    assert_eq!(
        runtime.tracker.stats().abandoned,
        1,
        "the timed-out job must still be running and abandoned"
    );
    let (captured, _guard) = test_support::capture();
    service.shutdown().await;
    let logs = captured.text();
    assert!(logs.contains("shutdown grace elapsed"), "logs were: {logs}");
    assert!(logs.contains("jobs=1"), "logs were: {logs}");
    release(runtime, &hook);
    wait_for_idle(runtime).await;
}
