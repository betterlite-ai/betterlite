use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, FixedOffset, NaiveDateTime};
use rstest::rstest;

use crate::spend_partitions::partition_bound::{
    CatalogPartition, PARTITION_BOUND_MAX_BYTES, PartitionBound, PartitionBoundError,
    decode_partition_bound, parse_partition_upper_bound, select_partitions_to_drop,
};

fn stamp(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").expect("a wall-clock stamp")
}

fn offset(raw: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(raw).expect("an offset timestamp")
}

fn partition(name: &str, bound: PartitionBound) -> CatalogPartition {
    CatalogPartition {
        name: name.to_owned(),
        bound,
    }
}

#[rstest]
#[case::plain(
    "FOR VALUES FROM ('2026-06-01 00:00:00') TO ('2026-06-02 00:00:00')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00"))
)]
#[case::milliseconds(
    "FOR VALUES FROM ('2026-06-01 00:00:00') TO ('2026-06-02 00:00:00.123')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00.123"))
)]
#[case::date_only(
    "FOR VALUES FROM ('2026-06-01') TO ('2026-06-02')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00"))
)]
#[case::iso_separator(
    "FOR VALUES FROM ('2026-06-01T00:00:00') TO ('2026-06-02T00:00:00')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00"))
)]
#[case::offset_hours(
    "FOR VALUES FROM ('2026-06-01 00:00:00+02') TO ('2026-06-02 00:00:00+02')",
    PartitionBound::Aware(offset("2026-06-02T00:00:00+02:00"))
)]
#[case::offset_minutes(
    "FOR VALUES FROM ('2026-06-01 00:00:00+02:30') TO ('2026-06-02 00:00:00+02:30')",
    PartitionBound::Aware(offset("2026-06-02T00:00:00+02:30"))
)]
#[case::utc(
    "FOR VALUES FROM ('2026-06-01 00:00:00+00') TO ('2026-06-02 00:00:00+00')",
    PartitionBound::Aware(offset("2026-06-02T00:00:00Z"))
)]
#[case::pinned_to_only(
    "TO ('2026-06-02 00:00:00')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00"))
)]
#[case::second_block(
    "TO ('broken' TO ('2026-06-02 00:00:00')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00"))
)]
#[case::empty_first_block(
    "TO ('') TO ('2026-06-02 00:00:00')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00"))
)]
#[case::nanoseconds_keep_pythons_microseconds(
    "FOR VALUES FROM ('2026-06-01 00:00:00') TO ('2026-06-02 00:00:00.123456789')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00.123456"))
)]
#[case::longer_fractions_read_like_python(
    "FOR VALUES FROM ('2026-06-01 00:00:00') TO ('2026-06-02 00:00:00.1234567890')",
    PartitionBound::Naive(stamp("2026-06-02 00:00:00.123456"))
)]
fn parses_the_pinned_upper_bound(#[case] expression: &str, #[case] expected: PartitionBound) {
    assert_eq!(parse_partition_upper_bound(expression), expected);
}

#[rstest]
#[case::plain("DEFAULT")]
#[case::lowercase("default")]
#[case::mixed_case("DeFaUlT")]
#[case::with_values("FOR VALUES FROM ('2026-06-01') TO ('2026-07-01') DEFAULT")]
#[case::unbounded("FOR VALUES FROM (MINVALUE) TO (MAXVALUE)")]
fn default_and_unbounded_bounds_are_unknown(#[case] expression: &str) {
    assert_eq!(
        parse_partition_upper_bound(expression),
        PartitionBound::Unknown
    );
}

#[rstest]
#[case::empty("")]
#[case::no_prefix("FOR VALUES FROM ('2026-06-01')")]
#[case::empty_upper("FOR VALUES FROM ('2026-06-01') TO ('')")]
#[case::missing_paren("FOR VALUES FROM ('2026-06-01') TO ('2026-06-02'")]
#[case::not_a_timestamp("FOR VALUES FROM ('2026-06-01') TO ('tomorrow')")]
#[case::impossible_day("FOR VALUES FROM ('2026-06-01') TO ('2026-02-30 00:00:00')")]
#[case::trailing_text("FOR VALUES FROM ('2026-06-01') TO ('2026-06-02' extra)")]
#[case::nested_timestamp("FOR VALUES FROM ('2026-06-01') TO ('2026-06-02 00:00:00'::timestamp)")]
#[case::basic_date("FOR VALUES FROM ('2026-06-01') TO ('20260602')")]
#[case::week_date("FOR VALUES FROM ('2026-06-01') TO ('2026-W23-1')")]
#[case::arbitrary_separator("FOR VALUES FROM ('2026-06-01') TO ('2026-06-02X00:00:00')")]
fn unparseable_bounds_are_unknown(#[case] expression: &str) {
    assert_eq!(
        parse_partition_upper_bound(expression),
        PartitionBound::Unknown
    );
}

#[test]
fn the_byte_ceiling_is_exact_and_rejects_non_utf8() {
    let expression = "TO ('2026-06-02 00:00:00')";
    let padding = PARTITION_BOUND_MAX_BYTES.saturating_sub(expression.len());
    let at_limit = format!("{expression}{}", " ".repeat(padding));
    assert_eq!(at_limit.len(), PARTITION_BOUND_MAX_BYTES);
    assert_eq!(
        decode_partition_bound(at_limit.as_bytes()),
        PartitionBound::Naive(stamp("2026-06-02 00:00:00"))
    );
    let over_limit = format!("{at_limit} ");
    assert_eq!(
        decode_partition_bound(over_limit.as_bytes()),
        PartitionBound::Unknown
    );
    assert_eq!(
        decode_partition_bound(&[0xff, 0xfe, 0x00]),
        PartitionBound::Unknown
    );
}

#[rstest]
#[case::before(stamp("2026-06-01 23:59:59.999"), true)]
#[case::exactly_at_the_cutoff(stamp("2026-06-02 00:00:00"), true)]
#[case::after(stamp("2026-06-02 00:00:00.001"), false)]
fn selects_naive_bounds_at_or_before_the_cutoff(
    #[case] upper: NaiveDateTime,
    #[case] dropped: bool,
) {
    let partitions = [
        partition("aged", PartitionBound::Naive(upper)),
        partition("default", PartitionBound::Unknown),
    ];
    let selected = select_partitions_to_drop(&partitions, stamp("2026-06-02 00:00:00"))
        .expect("the partition selection");
    let expected = if dropped {
        vec!["aged".to_owned()]
    } else {
        Vec::new()
    };
    assert_eq!(selected, expected);
}

#[test]
fn an_aware_upper_bound_fails_the_selection() {
    let partitions = [
        partition("naive", PartitionBound::Naive(stamp("2026-05-01 00:00:00"))),
        partition(
            "aware",
            PartitionBound::Aware(offset("2026-06-01T00:00:00+02:00")),
        ),
        partition("default", PartitionBound::Unknown),
    ];
    let error = select_partitions_to_drop(&partitions, stamp("2026-06-02 00:00:00"))
        .expect_err("an offset upper bound");
    assert!(matches!(error, PartitionBoundError::OffsetBound { .. }));
    assert!(error.to_string().contains("2026-06-01 00:00:00"));
    assert!(error.to_string().contains("+02:00"));
    assert!(error.to_string().contains("2026-06-02 00:00:00"));
}

#[rstest]
fn replays_the_partition_bound_corpus(#[files("fuzz/corpus/partition_bound/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("a partition bound corpus file");
    let _bound = decode_partition_bound(&bytes);
}
