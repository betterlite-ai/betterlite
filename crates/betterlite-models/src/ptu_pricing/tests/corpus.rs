use std::fs;
use std::path::Path;

use chrono::{DateTime, TimeZone as _, Utc};
use rstest::rstest;

use crate::savings_evidence::billed_at::decode_billed_at;

#[rstest]
#[case::date_only("2026-01-01", "2026-01-01T00:00:00Z")]
#[case::zulu("2026-01-01T10:00:00Z", "2026-01-01T10:00:00Z")]
#[case::offset("2026-01-01T10:00:00+05:30", "2026-01-01T04:30:00Z")]
#[case::naive("2026-01-01T10:00:00", "2026-01-01T10:00:00Z")]
#[case::fraction("2026-01-01T10:00:00.123456Z", "2026-01-01T10:00:00.123456Z")]
#[case::week_date("2026-W01-4", "2026-01-01T00:00:00Z")]
#[case::comma_fraction("2026-01-01T10:00:00,5", "2026-01-01T10:00:00.500000Z")]
fn ptu_instant_reads_the_fromisoformat_forms(#[case] text: &str, #[case] expected: &str) {
    let expected_instant = DateTime::parse_from_rfc3339(expected).unwrap();
    assert_eq!(
        decode_billed_at(text),
        Some(expected_instant.with_timezone(&Utc))
    );
}

#[rstest]
#[case::word("soon")]
#[case::month_out_of_range("2026-13-01")]
#[case::day_out_of_range("2026-02-30")]
#[case::bare_time("10:00:00")]
#[case::empty("")]
#[case::trailing_junk("2026-01-01T00:00:00Zx")]
fn ptu_instant_rejects_invalid_text(#[case] text: &str) {
    assert_eq!(decode_billed_at(text), None);
}

#[rstest]
fn replays_the_ptu_instant_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/ptu_instant");
    for entry in fs::read_dir(directory).expect("the ptu_instant corpus directory exists") {
        let path = entry.expect("a corpus entry is readable").path();
        if let Ok(text) = fs::read_to_string(path) {
            let _outcome = decode_billed_at(&text);
        }
    }
}

#[test]
fn ptu_instant_midnight_is_utc() {
    assert_eq!(
        decode_billed_at("2026-01-01"),
        Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap())
    );
}
