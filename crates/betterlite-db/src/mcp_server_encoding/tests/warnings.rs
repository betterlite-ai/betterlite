#![cfg(test)]

use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::{
    decrypt_cipher, decrypt_global_env_var_values, decrypt_returned_row_env_vars,
    decrypt_stored_env_var_values, entries, request, row, typed_env_var,
};
use crate::managed_resource_rows::mcp::McpEnvVarScope;

#[derive(Clone, Default)]
struct WarnLogs(Arc<Mutex<Vec<String>>>);

struct EventFields(String);

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push_str(field.name());
        self.0.push('=');
        self.0.push_str(value);
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

impl<Inner: Subscriber> Layer<Inner> for WarnLogs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut fields = EventFields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
}

fn warnings_during<Run: FnOnce()>(run: Run) -> Vec<String> {
    let logs = WarnLogs::default();
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(logs.clone()));
    tracing::dispatcher::with_default(&dispatch, run);
    logs.0.lock().unwrap().clone()
}

#[test]
fn an_undecryptable_typed_global_env_var_warns_with_its_name() {
    let mut env_vars = vec![
        typed_env_var("TOKEN", "not-a-ciphertext", McpEnvVarScope::Global),
        typed_env_var("PLACEHOLDER", "not-a-ciphertext", McpEnvVarScope::User),
    ];
    let warnings = warnings_during(|| {
        decrypt_global_env_var_values(&mut env_vars, &decrypt_cipher());
    });
    assert_eq!(warnings, vec!["env_var=TOKEN".to_owned()]);
}

#[test]
fn an_undecryptable_stored_global_env_var_warns_with_its_name() {
    let mut returned = row(
        r#"{"env_vars":[{"name":"TOKEN","value":"not-a-ciphertext","scope":"global"},{"name":"PLACEHOLDER","value":"x","scope":"user"}]}"#,
    );
    let warnings = warnings_during(|| {
        decrypt_returned_row_env_vars(&mut returned, &decrypt_cipher()).unwrap();
    });
    assert_eq!(warnings, vec!["env_var=TOKEN".to_owned()]);
}

#[test]
fn an_undecryptable_stored_global_env_var_without_a_name_warns_with_an_empty_name() {
    let mut stored = entries(r#"[{"value":"not-a-ciphertext"},{"name":7,"value":"nope"}]"#);
    let warnings = warnings_during(|| {
        decrypt_stored_env_var_values(&mut stored, &decrypt_cipher());
    });
    assert_eq!(warnings, vec!["env_var=".to_owned(), "env_var=".to_owned()]);
}

#[test]
fn a_request_prints_no_field_values_in_debug_output() {
    let server = request(
        r#"{"server_name":"srv","credentials":{"client_secret":"s3cret"},"env_vars":[{"name":"A","value":"s3cret"}]}"#,
    );
    assert_eq!(format!("{server:?}"), "McpServerRequest { .. }");
    let env_var = server.env_vars.unwrap().remove(0);
    assert_eq!(format!("{env_var:?}"), "EnvVarRequest { .. }");
}
