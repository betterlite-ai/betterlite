use betterlite_config::loader::ConfigError;
use betterlite_redact::error::RedactionError;

use crate::otel::config::OTelConfigError;

#[derive(Debug, thiserror::Error)]
pub enum ProxyRuntimeError {
    #[error("the OTel settings could not be built from the config: {0}")]
    Settings(#[from] OTelConfigError),
    #[error("the OTel V2 switch could not be read: {source}")]
    V2Flag {
        #[from]
        source: ConfigError,
    },
    #[error("the {variable} value must be a non-negative integer")]
    AttributeLimit { variable: &'static str },
    #[error("the OTel redactor could not be built: {0}")]
    Redactor(#[from] RedactionError),
}
