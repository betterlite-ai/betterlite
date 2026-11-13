#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::support::{JsonOutput, at_offset, record, redactor, utc};
use crate::database_diagnostics::json_formatter::JsonFormatter;
use crate::database_diagnostics::legacy::TextFormatter;
use crate::database_diagnostics::message::{LogArguments, LogValue};
use crate::database_diagnostics::record::{ExceptionRecord, LogRecord};
use crate::database_diagnostics::timestamp::format_time;

fn json_output(record: &LogRecord<'_>) -> JsonOutput {
    let local = utc();
    let redactor = redactor();
    let formatter = JsonFormatter {
        local_time: &local,
        redactor: &redactor,
    };
    let rendered = formatter.format(record).unwrap();
    serde_json::from_str(&rendered).unwrap()
}

#[test]
fn json_three_fields_only() {
    let parsed = json_output(&record(1_700_000_000.0, "INFO", "beat"));
    assert_eq!(parsed.message, "beat");
    assert_eq!(parsed.level, "INFO");
    assert_eq!(parsed.timestamp, "2023-11-14T22:13:20");
    assert_eq!(parsed.stacktrace, None);
    let local = utc();
    let redactor = redactor();
    let rendered = JsonFormatter {
        local_time: &local,
        redactor: &redactor,
    }
    .format(&record(1_700_000_000.0, "INFO", "beat"))
    .unwrap();
    assert!(!rendered.contains("stacktrace"));
    assert!(!rendered.contains("logger"));
}

#[test]
fn message_arguments_formatted() {
    let mut positional = record(1_700_000_000.0, "INFO", "worker %s beat %d times");
    positional.arguments = LogArguments::Positional(vec![
        LogValue::Text("alpha".to_owned()),
        LogValue::Integer(3),
    ]);
    assert_eq!(
        json_output(&positional).message,
        "worker alpha beat 3 times"
    );
    let mut named = record(1_700_000_000.0, "INFO", "count %(count)d for %(worker)s");
    named.arguments = LogArguments::Named(BTreeMap::from([
        ("count".to_owned(), LogValue::Integer(2)),
        ("worker".to_owned(), LogValue::Text("alpha".to_owned())),
    ]));
    assert_eq!(json_output(&named).message, "count 2 for alpha");
    let mut repeated = record(1_700_000_000.0, "INFO", "100%% %r");
    repeated.arguments = LogArguments::Positional(vec![LogValue::Text("done".to_owned())]);
    assert_eq!(json_output(&repeated).message, "100% 'done'");
}

#[test]
fn local_iso_no_zone() {
    let local = at_offset(19_800);
    let redactor = redactor();
    let formatter = JsonFormatter {
        local_time: &local,
        redactor: &redactor,
    };
    let rendered = formatter
        .format(&record(1_700_000_000.125, "INFO", "beat"))
        .unwrap();
    let parsed: JsonOutput = serde_json::from_str(&rendered).unwrap();
    assert_eq!(parsed.timestamp, "2023-11-15T03:43:20.125000");
    assert!(!parsed.timestamp.contains('+'));
    assert!(!parsed.timestamp.ends_with('Z'));
}

#[rstest]
#[case::whole_second(1_700_000_000.0, "2023-11-14T22:13:20")]
#[case::microseconds(1_700_000_000.125, "2023-11-14T22:13:20.125000")]
#[case::single_microsecond(1_700_000_000.000_001, "2023-11-14T22:13:20.000001")]
fn iso_fraction_only_when_nonzero(#[case] created: f64, #[case] expected: &str) {
    let local = utc();
    assert_eq!(format_time(&local, created, None).unwrap(), expected);
}

#[test]
fn datefmt_ignored() {
    let local = at_offset(19_800);
    let default = format_time(&local, 1_700_000_000.125, None).unwrap();
    let configured = format_time(&local, 1_700_000_000.125, Some("%H:%M")).unwrap();
    assert_eq!(default, "2023-11-15T03:43:20.125000");
    assert_eq!(default, configured);
}

#[test]
fn stacktrace_only_with_exception() {
    assert_eq!(
        json_output(&record(1_700_000_000.0, "INFO", "beat")).stacktrace,
        None
    );
    let mut failed = record(1_700_000_000.0, "ERROR", "beat");
    failed.exception = Some(ExceptionRecord {
        kind: "ValueError",
        message: "boom",
    });
    assert_eq!(
        json_output(&failed).stacktrace,
        Some("ValueError: boom".to_owned())
    );
}

#[test]
fn json_unicode_control_strings() {
    let local = utc();
    let redactor = redactor();
    let message = "héllo\nworld\u{1}";
    let rendered = JsonFormatter {
        local_time: &local,
        redactor: &redactor,
    }
    .format(&record(1_700_000_000.0, "INFO", message))
    .unwrap();
    assert!(rendered.contains("héllo"));
    assert!(rendered.contains("\\n"));
    assert!(rendered.contains("\\u0001"));
    let parsed: JsonOutput = serde_json::from_str(&rendered).unwrap();
    assert_eq!(parsed.message, message);
}

#[test]
fn legacy_local_millisecond_layout() {
    let local = at_offset(19_800);
    let redactor = redactor();
    let formatter = TextFormatter {
        local_time: &local,
        redactor: &redactor,
    };
    let exact = formatter
        .format(&record(1_700_000_000.125, "INFO", "message"))
        .unwrap();
    assert_eq!(
        exact,
        "2023-11-15 03:43:20,125 - litellm_proxy_extras - INFO - message"
    );
    let truncated = formatter
        .format(&record(1_700_000_000.123, "INFO", "message"))
        .unwrap();
    assert_eq!(
        truncated,
        "2023-11-15 03:43:20,122 - litellm_proxy_extras - INFO - message"
    );
    let padded = formatter
        .format(&record(1_700_000_000.005, "INFO", "message"))
        .unwrap();
    assert_eq!(
        padded,
        "2023-11-15 03:43:20,005 - litellm_proxy_extras - INFO - message"
    );
    let mut failed = record(1_700_000_000.125, "ERROR", "message");
    failed.exception = Some(ExceptionRecord {
        kind: "ValueError",
        message: "boom",
    });
    assert_eq!(
        formatter.format(&failed).unwrap(),
        "2023-11-15 03:43:20,125 - litellm_proxy_extras - ERROR - message\nValueError: boom"
    );
}

#[test]
fn native_gateway_stderr_stays_json() {
    let local = utc();
    let redactor = redactor();
    let entry = record(1_700_000_000.0, "INFO", "beat");
    let text = TextFormatter {
        local_time: &local,
        redactor: &redactor,
    }
    .format(&entry)
    .unwrap();
    let json = JsonFormatter {
        local_time: &local,
        redactor: &redactor,
    }
    .format(&entry)
    .unwrap();
    assert!(!text.starts_with('{'));
    assert!(json.starts_with('{'));
    let parsed: JsonOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.message, "beat");
}
