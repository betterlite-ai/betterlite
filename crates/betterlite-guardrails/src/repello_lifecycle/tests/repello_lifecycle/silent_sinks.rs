use std::sync::Arc;

use betterlite_guardrails::repello_lifecycle::stream::{SilentConsole, SilentLogger};
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;

fn hooks(verbose: bool) -> VerboseHooks<SilentLogger, SilentConsole> {
    VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(verbose),
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    }
}

#[test]
fn a_debug_line_reaches_no_log() {
    let result = hooks(false).main_print_verbose(Payload::Bool(true));
    assert!(result.logs.is_empty());
}

#[test]
fn an_info_or_error_line_reaches_no_log() {
    let hooks = hooks(false);
    let info = hooks.utils_print_verbose(Payload::Bool(true), "INFO", &Payload::Bool(false));
    let error = hooks.utils_print_verbose(Payload::Bool(true), "ERROR", &Payload::Bool(false));
    assert!(info.logs.is_empty());
    assert!(error.logs.is_empty());
}

#[test]
fn the_guardrail_is_never_active_for_verbose_logging() {
    assert!(!hooks(false).is_active());
}

#[test]
fn a_verbose_run_prints_nothing_to_the_console() {
    let result =
        hooks(true).utils_print_verbose(Payload::Bool(true), "UNKNOWN", &Payload::Bool(false));
    assert!(result.console.is_empty());
}
