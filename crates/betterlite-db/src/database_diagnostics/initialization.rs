use crate::database_diagnostics::level::{LevelError, LogLevel, parse_level};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticSettings {
    pub level: String,
    pub json: bool,
}

impl Default for DiagnosticSettings {
    fn default() -> Self {
        Self {
            level: "INFO".to_owned(),
            json: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticFormat {
    Json,
    Text,
}

pub trait DiagnosticLogger {
    fn has_handler(&self) -> bool;
    fn install_handler(&mut self, format: DiagnosticFormat);
    fn set_level(&mut self, level: LogLevel);
}

// a level error is raised only after the handler is installed, so a repeated call sees the handler and skips it
pub fn initialize_logging(
    settings: &DiagnosticSettings,
    logger: &mut dyn DiagnosticLogger,
) -> Result<(), LevelError> {
    if logger.has_handler() {
        return Ok(());
    }
    logger.install_handler(if settings.json {
        DiagnosticFormat::Json
    } else {
        DiagnosticFormat::Text
    });
    logger.set_level(parse_level(&settings.level)?);
    Ok(())
}
