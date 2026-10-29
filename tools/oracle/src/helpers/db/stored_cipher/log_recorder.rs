use std::sync::Mutex;

use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use serde_json::{Value, json};

pub(super) struct LogRecorder<'step> {
    pub(super) field: &'step str,
    pub(super) records: Mutex<Vec<Value>>,
}

impl SecretDiagnostics for LogRecorder<'_> {
    fn record(&self, level: Level, event: &SecretEvent<'_>) {
        let message = event.message();
        let names_field = !self.field.is_empty() && message.contains(self.field);
        let salt_advice =
            message.contains("https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key");
        let level_name = match level {
            Level::Debug => "debug",
            Level::Error => "error",
        };
        if let Ok(mut records) = self.records.lock() {
            records.push(json!({"level": level_name, "names_field": names_field, "salt_advice": salt_advice}));
        }
    }
}
