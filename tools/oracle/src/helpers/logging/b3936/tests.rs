#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_logging::datadog_llm_privacy::datadog_budget_reset_decoder::{
    BudgetResetError, decode_budget_reset,
};
use rstest::rstest;

fn expected(name: &str) -> Option<Result<String, BudgetResetError>> {
    let decoded = match name {
        "naive-datetime.txt" | "date-only.txt" | "utc-z.txt" => {
            Ok("2026-01-01T00:00:00+00:00".to_owned())
        }
        "naive-space.txt" => Ok("2026-01-01T12:34:56+00:00".to_owned()),
        "utc-fraction-z.txt" | "fraction-long.txt" => {
            Ok("2026-01-01T00:00:00.123456+00:00".to_owned())
        }
        "at-limit.txt" => Ok("2026-01-01T00:00:00.111111+00:00".to_owned()),
        "offset-plus.txt" => Ok("2026-01-01T00:00:00+02:00".to_owned()),
        "offset-minus.txt" => Ok("2026-01-01T00:00:00-05:30".to_owned()),
        "past-limit.txt" => Err(BudgetResetError::TooLong { length: 65 }),
        "invalid-text.txt" | "truncated.txt" | "empty.txt" => Err(BudgetResetError::Invalid),
        _ => return None,
    };
    Some(decoded)
}

#[rstest]
fn replays_the_datadog_budget_reset_decoder_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/datadog_budget_reset_decoder");
    let mut seen = 0_usize;
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let text = fs::read_to_string(&path).unwrap();
        seen = seen.saturating_add(1);
        let Some(decoded) = expected(&name) else {
            panic!("unexpected corpus file {name}");
        };
        assert_eq!(decode_budget_reset(&text), decoded, "{name}");
    }
    assert!(seen > 0, "the budget reset decoder corpus is empty");
}
