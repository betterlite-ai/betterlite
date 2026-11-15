#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    NotSet,
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LevelError {
    #[error("LITELLM_LOG is not a known log level")]
    Unknown,
}

// the source uppercases the raw value and passes it to logging.setLevel, which takes no whitespace
pub fn parse_level(text: &str) -> Result<LogLevel, LevelError> {
    match text.to_uppercase().as_str() {
        "NOTSET" => Ok(LogLevel::NotSet),
        "DEBUG" => Ok(LogLevel::Debug),
        "INFO" => Ok(LogLevel::Info),
        "WARN" | "WARNING" => Ok(LogLevel::Warning),
        "ERROR" => Ok(LogLevel::Error),
        "FATAL" | "CRITICAL" => Ok(LogLevel::Critical),
        _ => Err(LevelError::Unknown),
    }
}
