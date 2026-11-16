use betterlite_config::sections::audit_events::Settings;
use betterlite_config::sections::settings::JsonValue;

use super::callbacks::AuditCallbackConfig;

pub trait AuditSettings: Send + Sync {
    fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool;
    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig];
    fn s3_audit_callback_params(&self) -> Option<&JsonValue>;
}

pub struct ConfiguredAuditSettings {
    pub config: Settings,
    pub callbacks: Vec<AuditCallbackConfig>,
}

impl AuditSettings for ConfiguredAuditSettings {
    fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool {
        self.config.is_audit_logging_enabled(explicit)
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &self.callbacks
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        self.config.s3_audit_callback_params.as_ref()
    }
}
