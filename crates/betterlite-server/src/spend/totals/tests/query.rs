use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::query::{LimitQuery, decode_spend_totals_query};

#[rstest]
#[case::empty(b"")]
#[case::key_only(b"api_key=sk-one")]
#[case::unrelated_parameters(b"offset=3&foo=bar")]
fn defaults_the_limit_to_absent(#[case] raw: &[u8]) {
    assert_eq!(decode_spend_totals_query(raw).limit, LimitQuery::Absent);
}

#[test]
fn reads_the_key_next_to_unrelated_parameters() {
    assert_eq!(decode_spend_totals_query(b"").api_key, None);
    assert_eq!(
        decode_spend_totals_query(b"offset=3&api_key=sk-one").api_key,
        Some("sk-one".to_owned())
    );
}

#[rstest]
#[case::plain(b"limit=5", 5)]
#[case::leading_zero(b"limit=05", 5)]
#[case::plus_sign(b"limit=%2B1", 1)]
#[case::negative_zero(b"limit=-0", 0)]
#[case::zero_fraction(b"limit=0001.000", 1)]
#[case::surrounding_space(b"limit=%2010%20", 10)]
#[case::plus_as_space(b"limit=+10", 10)]
#[case::single_underscore(b"limit=1_0", 10)]
#[case::grouped_underscores(b"limit=1_000", 1000)]
#[case::underscore_before_the_fraction(b"limit=1_0.0", 10)]
#[case::negative(b"limit=-3", -3)]
fn accepts_the_integer_text_pydantic_accepts(#[case] raw: &[u8], #[case] expected: i64) {
    assert_eq!(
        decode_spend_totals_query(raw).limit,
        LimitQuery::Value(expected)
    );
}

#[rstest]
#[case::empty(b"limit=")]
#[case::fraction(b"limit=5.5")]
#[case::exponent(b"limit=1e3")]
#[case::trailing_point(b"limit=1.")]
#[case::leading_point(b"limit=.0")]
#[case::small_fraction(b"limit=1.000001")]
#[case::trailing_underscore(b"limit=1_")]
#[case::leading_underscore(b"limit=_1")]
#[case::double_underscore(b"limit=1__0")]
#[case::underscore_in_fraction(b"limit=5.0_0")]
#[case::underscore_after_the_point(b"limit=1._0")]
#[case::underscore_ends_fraction(b"limit=1.0_")]
#[case::hex(b"limit=0x10")]
#[case::infinity(b"limit=Infinity")]
#[case::nan(b"limit=NaN")]
#[case::double_point(b"limit=1.0.0")]
#[case::unicode_digits(b"limit=%D9%A1%D9%A2")]
#[case::inner_space(b"limit=1%200")]
#[case::lone_sign(b"limit=-")]
fn rejects_the_integer_text_pydantic_rejects(#[case] raw: &[u8]) {
    assert_eq!(decode_spend_totals_query(raw).limit, LimitQuery::Invalid);
}

#[rstest]
#[case::maximum(b"limit=9223372036854775807", LimitQuery::Value(i64::MAX))]
#[case::minimum(b"limit=-9223372036854775808", LimitQuery::Value(i64::MIN))]
#[case::grouped_maximum(b"limit=9_223_372_036_854_775_807", LimitQuery::Value(i64::MAX))]
#[case::past_maximum(b"limit=9223372036854775808", LimitQuery::Invalid)]
#[case::past_minimum(b"limit=-9223372036854775809", LimitQuery::Invalid)]
fn keeps_the_integer_range(#[case] raw: &[u8], #[case] expected: LimitQuery) {
    assert_eq!(decode_spend_totals_query(raw).limit, expected);
}

#[test]
fn keeps_the_last_repeated_parameter() {
    let query = decode_spend_totals_query(b"api_key=first&limit=1&api_key=second&limit=2");
    assert_eq!(query.api_key, Some("second".to_owned()));
    assert_eq!(query.limit, LimitQuery::Value(2));
}

#[rstest]
#[case::plain(b"api_key=sk-one", "sk-one")]
#[case::percent_escaped(b"api_key=sk%2Done", "sk-one")]
#[case::reserved_characters(b"api_key=a%26b%3Dc%20d", "a&b=c d")]
#[case::plus_as_space(b"api_key=a+b", "a b")]
#[case::empty(b"api_key=", "")]
fn decodes_the_key_parameter(#[case] raw: &[u8], #[case] expected: &str) {
    assert_eq!(
        decode_spend_totals_query(raw).api_key.as_deref(),
        Some(expected)
    );
}

#[rstest]
fn replays_the_spend_totals_query_corpus(
    #[files("fuzz/corpus/spend_totals_query/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let query = decode_spend_totals_query(&bytes);
    if let Some(api_key) = query.api_key.as_deref() {
        let encoded = form_urlencoded::byte_serialize(api_key.as_bytes()).collect::<String>();
        let again = decode_spend_totals_query(format!("api_key={encoded}").as_bytes());
        assert_eq!(again.api_key.as_deref(), Some(api_key));
    }
    match query.limit {
        LimitQuery::Value(limit) => {
            let again = decode_spend_totals_query(format!("limit={limit}").as_bytes());
            assert_eq!(again.limit, LimitQuery::Value(limit));
        }
        LimitQuery::Absent => {
            assert_eq!(decode_spend_totals_query(b"").limit, LimitQuery::Absent);
        }
        LimitQuery::Invalid => {}
    }
}
