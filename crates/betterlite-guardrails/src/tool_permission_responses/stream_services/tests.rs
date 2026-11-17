#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use rstest::rstest;

use super::{SilentConsole, SilentLogger};

fn silent_hooks() -> VerboseHooks<SilentLogger, SilentConsole> {
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor patterns compile"),
        ),
    }
}

#[rstest]
#[case::debug("DEBUG")]
#[case::info("INFO")]
#[case::error("ERROR")]
fn silent_logger_keeps_every_level_in_the_hook_result(#[case] level: &str) {
    let result = silent_hooks().utils_print_verbose(
        Payload::String("hello".to_owned()),
        level,
        &Payload::Bool(false),
    );
    assert_eq!(result.logs.len(), 1);
}

#[test]
fn silent_logger_reports_no_verbose_output() {
    assert!(!silent_hooks().is_active());
}

#[test]
fn silent_console_accepts_a_payload() {
    let hooks = VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(true),
        redactor: Arc::new(
            Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor patterns compile"),
        ),
    };
    let result = hooks.main_print_verbose(Payload::String("hello".to_owned()));
    assert_eq!(result.console.len(), 1);
}
