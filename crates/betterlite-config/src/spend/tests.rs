#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_db::spend_batches::StatementLimits;
use rstest::rstest;

use super::{
    ENV_MAX_BYTES, ENV_MAX_ROWS, SpendStatementLimitsError, decode_spend_statement_limits,
    statement_limits,
};
use crate::loader::ConfigError;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(pairs: &[(&str, &str)]) -> StatementLimits {
    statement_limits(&environment(pairs)).unwrap()
}

const fn limits(max_bytes: usize, max_rows: usize) -> StatementLimits {
    StatementLimits {
        max_bytes,
        max_rows,
    }
}

#[test]
fn loads_spend_statement_limits() {
    assert_eq!(load(&[]), limits(2_000_000, 100));
    assert_eq!(
        load(&[(ENV_MAX_BYTES, "4096"), (ENV_MAX_ROWS, "7")]),
        limits(4096, 7)
    );
    assert_eq!(
        load(&[(ENV_MAX_BYTES, "500000"), (ENV_MAX_ROWS, "25")]),
        limits(500_000, 25)
    );
    assert_eq!(
        load(&[(ENV_MAX_BYTES, "0"), (ENV_MAX_ROWS, "-5")]),
        limits(1, 1)
    );
    assert_eq!(
        load(&[(ENV_MAX_BYTES, ""), (ENV_MAX_ROWS, "  ")]),
        limits(2_000_000, 100)
    );
    let error = statement_limits(&environment(&[(ENV_MAX_ROWS, "not-a-number")])).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::Environment {
            source: betterlite_env::EnvError::Number { .. }
        }
    ));
    assert!(!error.to_string().contains("not-a-number"));
    assert!(matches!(
        statement_limits(&environment(&[(ENV_MAX_BYTES, "12.5")])),
        Err(ConfigError::Environment {
            source: betterlite_env::EnvError::Number { .. }
        })
    ));
}

#[rstest]
#[case::zero("0", 1)]
#[case::negative("-9", 1)]
#[case::one("1", 1)]
#[case::two("2", 2)]
#[case::surrounding_space(" 64 ", 64)]
#[case::plus_sign("+9", 9)]
fn clamps_a_limit_to_at_least_one(#[case] text: &str, #[case] expected: usize) {
    let loaded = load(&[(ENV_MAX_BYTES, text), (ENV_MAX_ROWS, text)]);
    assert_eq!(loaded, limits(expected, expected));
}

#[test]
fn decodes_limits_from_a_stored_row() {
    let decoded = decode_spend_statement_limits(
        br#"{"environment":{"SPEND_LOG_WRITE_BATCH_MAX_BYTES":"4096","SPEND_LOG_WRITE_BATCH_MAX_ROWS":"-1"}}"#,
    )
    .unwrap();
    assert_eq!(decoded, limits(4096, 1));
    assert_eq!(
        decode_spend_statement_limits(b"{}").unwrap(),
        limits(2_000_000, 100)
    );
    assert!(matches!(
        decode_spend_statement_limits(b"not json").unwrap_err(),
        SpendStatementLimitsError::Json(_)
    ));
    assert!(matches!(
        decode_spend_statement_limits(br#"{"environment":{"SPEND_LOG_WRITE_BATCH_MAX_ROWS":"x"}}"#),
        Err(SpendStatementLimitsError::Config(_))
    ));
}

#[rstest]
fn replays_the_spend_statement_limits_corpus(
    #[files("fuzz/corpus/spend_statement_limits/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(limits) = decode_spend_statement_limits(&bytes) else {
        return;
    };
    assert!(limits.max_bytes >= 1);
    assert!(limits.max_rows >= 1);
}
