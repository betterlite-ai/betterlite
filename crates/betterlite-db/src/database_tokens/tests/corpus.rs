#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::database_tokens::expiry::{ExpiryError, TokenAuthKind, decode_database_token_expiry};
use crate::database_tokens::tests::support::capture;

enum Expectation {
    Value,
    Silent,
    Debug,
    Overflow,
}

fn expectation(name: &str) -> Expectation {
    if name.contains("-valid") {
        Expectation::Value
    } else if name.contains("-silent") {
        Expectation::Silent
    } else if name.contains("-debug") {
        Expectation::Debug
    } else if name.contains("-overflow") {
        Expectation::Overflow
    } else {
        panic!("the corpus file {name} names no expectation")
    }
}

fn replay_file(path: &Path) -> Option<String> {
    let name = path.file_name().and_then(|name| name.to_str())?.to_owned();
    let kind = if name.starts_with("rds-") {
        TokenAuthKind::Rds
    } else {
        TokenAuthKind::Azure
    };
    let bytes = fs::read(path).ok()?;
    let (result, logs) = capture(|| decode_database_token_expiry(&bytes, kind));
    let matched = match expectation(&name) {
        Expectation::Value => matches!(result, Ok(Some(_))) && logs.is_empty(),
        Expectation::Silent => matches!(result, Ok(None)) && logs.is_empty(),
        Expectation::Debug => matches!(result, Ok(None)) && !logs.is_empty(),
        Expectation::Overflow => matches!(result, Err(ExpiryError::TimestampOutOfRange)),
    };
    (!matched).then(|| format!("{name}: {result:?} {logs:?}"))
}

#[test]
fn replays_the_database_token_expiry_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/database_token_expiry");
    let paths: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("the database token expiry corpus exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    let mismatches: Vec<String> = paths.iter().filter_map(|path| replay_file(path)).collect();
    assert!(!paths.is_empty());
    assert_eq!(mismatches, Vec::<String>::new());
}
