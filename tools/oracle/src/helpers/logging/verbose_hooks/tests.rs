#![cfg(test)]

use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use betterlite_logging::verbose_hooks::LoggerSink;
use betterlite_redact::structured::Payload;

use super::FixtureLogger;

#[test]
fn fixture_logger_records_the_level_of_the_method_called() {
    let records = Arc::new(Cell::new(Vec::new()));
    let logger = FixtureLogger {
        fails: false,
        debug_enabled: false,
        records: Arc::clone(&records),
    };
    let payload = Payload::String("message".to_owned());
    assert!(logger.info(&payload));
    assert!(logger.error(&payload));
    assert!(logger.debug(&payload));
    assert_eq!(
        records
            .take()
            .iter()
            .map(|record| record.level.as_str())
            .collect::<Vec<_>>(),
        ["INFO", "ERROR", "DEBUG"]
    );
    let failed_records = Arc::new(Cell::new(Vec::new()));
    let failing = FixtureLogger {
        fails: true,
        debug_enabled: false,
        records: Arc::clone(&failed_records),
    };
    assert!(!failing.error(&payload));
    assert!(failed_records.take().is_empty());
}

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_verbose_hooks() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("verbose_hooks"))
        })
        .map(|path| fs::read_to_string(path).expect("seed should be readable"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case = serde_saphyr::from_str::<HelperCase>(source).expect("seed should parse");
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && case.call.operation == "verbose_hooks"
            && case.call.input.get("function").is_some()
    }));
}
