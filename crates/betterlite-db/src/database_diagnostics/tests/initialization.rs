#![cfg(test)]

use rstest::rstest;

use super::support::RecordingLogger;
use crate::database_diagnostics::initialization::{
    DiagnosticFormat, DiagnosticSettings, initialize_logging,
};
use crate::database_diagnostics::level::{LevelError, LogLevel};

fn settings(json: bool, level: &str) -> DiagnosticSettings {
    DiagnosticSettings {
        level: level.to_owned(),
        json,
    }
}

#[test]
fn existing_handlers_untouched() {
    let mut logger = RecordingLogger {
        formats: vec![DiagnosticFormat::Text],
        levels: Vec::new(),
    };
    initialize_logging(&settings(true, "DEBUG"), &mut logger).unwrap();
    assert_eq!(logger.formats, vec![DiagnosticFormat::Text]);
    assert!(logger.levels.is_empty());
}

#[test]
fn initialization_once() {
    let mut logger = RecordingLogger::default();
    initialize_logging(&settings(true, "debug"), &mut logger).unwrap();
    initialize_logging(&settings(true, "debug"), &mut logger).unwrap();
    assert_eq!(logger.formats, vec![DiagnosticFormat::Json]);
    assert_eq!(logger.levels, vec![LogLevel::Debug]);
}

#[test]
fn invalid_level_leaves_installed_handler() {
    let mut logger = RecordingLogger::default();
    let outcome = initialize_logging(&settings(true, "10"), &mut logger);
    assert_eq!(outcome, Err(LevelError::Unknown));
    assert_eq!(logger.formats, vec![DiagnosticFormat::Json]);
    assert!(logger.levels.is_empty());
}

#[test]
fn repeated_init_after_level_error_skips_validation() {
    let mut logger = RecordingLogger::default();
    assert!(initialize_logging(&settings(false, ""), &mut logger).is_err());
    initialize_logging(&settings(false, ""), &mut logger).unwrap();
    assert_eq!(logger.formats, vec![DiagnosticFormat::Text]);
    assert!(logger.levels.is_empty());
}

#[test]
fn existing_handlers_skip_level_validation() {
    let mut logger = RecordingLogger {
        formats: vec![DiagnosticFormat::Json],
        levels: Vec::new(),
    };
    initialize_logging(&settings(false, "not-a-level"), &mut logger).unwrap();
    assert_eq!(logger.formats, vec![DiagnosticFormat::Json]);
    assert!(logger.levels.is_empty());
}

#[test]
fn default_settings_use_info_and_text() {
    let mut logger = RecordingLogger::default();
    initialize_logging(&DiagnosticSettings::default(), &mut logger).unwrap();
    assert_eq!(logger.formats, vec![DiagnosticFormat::Text]);
    assert_eq!(logger.levels, vec![LogLevel::Info]);
}

#[rstest]
#[case::notset("NoTsEt", LogLevel::NotSet)]
#[case::debug("debug", LogLevel::Debug)]
#[case::info("Info", LogLevel::Info)]
#[case::warn("warn", LogLevel::Warning)]
#[case::warning("WARNING", LogLevel::Warning)]
#[case::error("error", LogLevel::Error)]
#[case::fatal("fatal", LogLevel::Critical)]
#[case::critical("Critical", LogLevel::Critical)]
fn initialization_sets_casefolded_level(#[case] level: &str, #[case] expected: LogLevel) {
    let mut logger = RecordingLogger::default();
    initialize_logging(&settings(false, level), &mut logger).unwrap();
    assert_eq!(logger.levels, vec![expected]);
}
