#![cfg(test)]

use super::support::{record, redactor, utc};
use crate::database_diagnostics::json_formatter::JsonFormatter;
use crate::database_diagnostics::legacy::TextFormatter;
use crate::database_diagnostics::record::ExceptionRecord;

const LEAKED: &str = "connect failed: password=hunter2";

#[test]
fn native_database_diagnostics_redact_exception_values() {
    let local = utc();
    let redactor = redactor();
    let mut failed = record(1_700_000_000.0, "ERROR", "beat");
    failed.exception = Some(ExceptionRecord {
        kind: "DatabaseError",
        message: LEAKED,
    });
    let json = JsonFormatter {
        local_time: &local,
        redactor: &redactor,
    }
    .format(&failed)
    .unwrap();
    assert!(json.contains("REDACTED"));
    assert!(!json.contains("hunter2"));
    let text = TextFormatter {
        local_time: &local,
        redactor: &redactor,
    }
    .format(&failed)
    .unwrap();
    assert!(text.contains("REDACTED"));
    assert!(!text.contains("hunter2"));
}
