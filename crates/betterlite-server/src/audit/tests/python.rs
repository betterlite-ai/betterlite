#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::callback_logger::error::LoggerError;

use super::support::{Workspace, record};
use crate::audit::python::audit_logger;
use crate::audit_events::payload::build_audit_log_payload;
use crate::python_hooks::PythonHooks;
use crate::python_hooks::slots::{ListKey, LoadedCallback, LoadedHook};

const CUSTOM_LOGGER: &str = r#"
class CustomLogger:
    async def async_log_audit_log_event(self, audit_log):
        pass
"#;

const MODULE: &str = r#"
from betterlite.integrations.custom_logger import CustomLogger

class Recorder(CustomLogger):
    async def async_log_audit_log_event(self, audit_log):
        if audit_log["changed_by"] != "admin":
            raise RuntimeError("unexpected actor: " + audit_log["changed_by"])

class Failing(CustomLogger):
    async def async_log_audit_log_event(self, audit_log):
        raise RuntimeError("callback saw " + audit_log["id"])

recorder = Recorder()
failing = Failing()

def handler(**kwargs):
    return "ran"
"#;

struct Fixture {
    _workspace: Workspace,
    module: String,
    service: PythonHooks,
}

impl Fixture {
    fn new() -> Self {
        let workspace = Workspace::new();
        workspace.file("betterlite/__init__.py", "");
        workspace.file("betterlite/integrations/__init__.py", "");
        workspace.file("betterlite/integrations/custom_logger.py", CUSTOM_LOGGER);
        let module = workspace.module("b065_python_audit", MODULE);
        let service = PythonHooks::new(Some(workspace.config_path.clone()));
        Self {
            _workspace: workspace,
            module,
            service,
        }
    }

    async fn load(&self, attribute: &str) -> LoadedHook {
        let entry = format!("{}.{attribute}", self.module);
        match self
            .service
            .load_callback(ListKey::AuditLogCallbacks, &entry)
            .await
        {
            Ok(LoadedCallback::Loaded(hook)) => hook,
            Ok(LoadedCallback::Named(name)) => panic!("the dotted entry loaded as {name}"),
            Err(error) => panic!("the callback loads: {error}"),
        }
    }
}

#[tokio::test]
async fn a_python_custom_logger_receives_the_audit_payload() {
    let fixture = Fixture::new();
    let hook = fixture.load("recorder").await;
    let logger = audit_logger(Arc::new(hook)).expect("the instance is a CustomLogger");
    assert_eq!(logger.callback_name(), "python_audit_log_callback");
    logger
        .async_log_audit_log_event(&build_audit_log_payload(&record()))
        .await
        .expect("the Python callback accepted the payload");
    fixture.service.shutdown().await;
}

#[tokio::test]
async fn a_python_failure_is_reported_as_a_delivery_error() {
    let fixture = Fixture::new();
    let hook = fixture.load("failing").await;
    let logger = audit_logger(Arc::new(hook)).expect("the instance is a CustomLogger");
    let audit = build_audit_log_payload(&record());
    let error = logger
        .async_log_audit_log_event(&audit)
        .await
        .expect_err("the Python callback raises");
    let LoggerError::Delivery { callback, message } = error;
    assert_eq!(callback, "python_audit_log_callback");
    assert!(message.contains("callback saw audit-1"), "{message}");
    fixture.service.shutdown().await;
}

#[tokio::test]
async fn a_loaded_function_or_class_is_not_an_audit_callback() {
    let fixture = Fixture::new();
    let function = fixture.load("handler").await;
    assert!(
        audit_logger(Arc::new(function)).is_none(),
        "a plain function is never an audit callback"
    );
    let class = fixture.load("Recorder").await;
    assert!(
        audit_logger(Arc::new(class)).is_none(),
        "a class is never an audit callback"
    );
    fixture.service.shutdown().await;
}
