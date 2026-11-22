use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::query::{TagReportQuery, decode_tag_report_query};

fn query_with_tags(tags: Option<&str>) -> TagReportQuery {
    tags.map_or_else(TagReportQuery::default, |text| {
        decode_tag_report_query(format!("tags={text}").as_bytes())
    })
}

#[rstest]
#[case::empty(b"")]
#[case::unrelated(b"offset=3&foo=bar")]
#[case::wrong_case(b"USER_ID=alice&TAGS=a")]
fn defaults_every_option(#[case] raw: &[u8]) {
    assert_eq!(decode_tag_report_query(raw), TagReportQuery::default());
}

#[test]
fn reads_every_scalar_option() {
    let query = decode_tag_report_query(
        b"user_id=alice&start_date=2024-01-01&end_date=2024-02-01&tags=a,b",
    );
    assert_eq!(query.user_id.as_deref(), Some("alice"));
    assert_eq!(query.start_date.as_deref(), Some("2024-01-01"));
    assert_eq!(query.end_date.as_deref(), Some("2024-02-01"));
    assert_eq!(query.tags.as_deref(), Some("a,b"));
}

#[test]
fn keeps_the_last_repeated_option() {
    let query = decode_tag_report_query(
        b"user_id=first&user_id=second&start_date=s1&start_date=s2&end_date=e1&end_date=e2&tags=a&tags=b",
    );
    assert_eq!(query.user_id.as_deref(), Some("second"));
    assert_eq!(query.start_date.as_deref(), Some("s2"));
    assert_eq!(query.end_date.as_deref(), Some("e2"));
    assert_eq!(query.tags.as_deref(), Some("b"));
}

#[rstest]
#[case::plain(b"user_id=alice", "alice")]
#[case::percent_escaped(b"user_id=alice%2Bb", "alice+b")]
#[case::plus_as_space(b"user_id=alice+b", "alice b")]
#[case::reserved_characters(b"user_id=a%26b%3Dc", "a&b=c")]
#[case::empty_value(b"user_id=", "")]
#[case::bare_name(b"user_id", "")]
#[case::invalid_escape_kept(b"user_id=%zz", "%zz")]
fn decodes_the_user_id(#[case] raw: &[u8], #[case] expected: &str) {
    assert_eq!(
        decode_tag_report_query(raw).user_id.as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::encoded_comma(b"tags=a%2Cb", "a,b")]
#[case::plus_as_space(b"tags=a+b", "a b")]
#[case::raw_commas(b"tags=a,,b,", "a,,b,")]
#[case::encoded_space(b"tags=a%20b", "a b")]
fn decodes_the_tags_verbatim(#[case] raw: &[u8], #[case] expected: &str) {
    assert_eq!(decode_tag_report_query(raw).tags.as_deref(), Some(expected));
}

#[rstest]
#[case(b"start_date=2024-1-5", "2024-1-5")]
#[case(b"end_date=2024%2F01%2F05", "2024/01/05")]
fn decodes_the_dates(#[case] raw: &[u8], #[case] expected: &str) {
    let query = decode_tag_report_query(raw);
    let value = query.start_date.or(query.end_date);
    assert_eq!(value.as_deref(), Some(expected));
}

#[rstest]
#[case::plain("a", &["a"])]
#[case::pair("a,b", &["a", "b"])]
#[case::space_after_comma("a, b", &["a", " b"])]
#[case::interior_empty("a,,b", &["a", "", "b"])]
#[case::only_commas(",", &["", ""])]
#[case::trailing_comma("a,", &["a", ""])]
#[case::underscore_is_ordinary("all_tags", &["all_tags"])]
fn splits_on_commas_without_trimming(#[case] text: &str, #[case] expected: &[&str]) {
    let listed: Vec<String> = expected.iter().map(|tag| (*tag).to_owned()).collect();
    assert_eq!(query_with_tags(Some(text)).selected_tags(), Some(listed));
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
#[case::single(Some("a"), Some(vec!["a"]))]
#[case::all_tags(Some("all-tags"), Some(vec!["all-tags"]))]
#[case::comma_only(Some(","), Some(vec!["", ""]))]
fn selects_the_tags_a_nonempty_string_lists(
    #[case] tags: Option<&str>,
    #[case] expected: Option<Vec<&str>>,
) {
    let wanted: Option<Vec<String>> =
        expected.map(|list| list.iter().map(|tag| (*tag).to_owned()).collect());
    assert_eq!(query_with_tags(tags).selected_tags(), wanted);
}

#[rstest]
#[case::absent(None, true)]
#[case::empty(Some(""), true)]
#[case::exact(Some("all-tags"), true)]
#[case::one_of_many(Some("a,all-tags"), true)]
#[case::underscore(Some("all_tags"), false)]
#[case::capitalized(Some("All-Tags"), false)]
#[case::trailing_space(Some("all-tags "), false)]
#[case::comma_only(Some(","), false)]
fn reads_every_tag_only_for_the_exact_element(#[case] tags: Option<&str>, #[case] expected: bool) {
    assert_eq!(query_with_tags(tags).selects_every_tag(), expected);
}

#[rstest]
fn replays_the_tag_report_query_corpus(#[files("fuzz/corpus/tag_report_query/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let query = decode_tag_report_query(&bytes);
    let mut rebuilt: Vec<String> = Vec::new();
    for (name, value) in [
        ("user_id", &query.user_id),
        ("start_date", &query.start_date),
        ("end_date", &query.end_date),
        ("tags", &query.tags),
    ] {
        if let Some(stored) = value {
            let encoded: String = form_urlencoded::byte_serialize(stored.as_bytes()).collect();
            rebuilt.push(format!("{name}={encoded}"));
        }
    }
    assert_eq!(decode_tag_report_query(rebuilt.join("&").as_bytes()), query);
    match query.selected_tags() {
        Some(selected) => assert_eq!(
            selected.join(","),
            query.tags.as_deref().unwrap_or_default()
        ),
        None => assert!(query.tags.as_deref().is_none_or(str::is_empty)),
    }
}
