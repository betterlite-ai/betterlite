use rstest::rstest;

use super::super::query::{
    SessionExpansionQuery, SessionQueryIssue, SessionQueryIssues, decode_session_query,
};

fn expansion(session_id: &str, page: i64, page_size: i64) -> SessionExpansionQuery {
    SessionExpansionQuery {
        session_id: session_id.to_owned(),
        page,
        page_size,
    }
}

fn only_issue(raw: &str) -> SessionQueryIssue {
    let issues = decode_session_query(Some(raw)).unwrap_err();
    assert_eq!(issues.detail.len(), 1, "{raw}");
    issues.detail.into_iter().next().unwrap()
}

#[test]
fn defaults_page_and_page_size() {
    assert_eq!(
        decode_session_query(Some("session_id=session-1")).unwrap(),
        expansion("session-1", 1, 50)
    );
}

// FastAPI reads session_id as a required string, and an empty one is still a value
#[test]
fn keeps_an_empty_session_id() {
    assert_eq!(
        decode_session_query(Some("session_id=")).unwrap(),
        expansion("", 1, 50)
    );
}

#[test]
fn the_last_repeated_parameter_wins() {
    assert_eq!(
        decode_session_query(Some("session_id=first&session_id=second")).unwrap(),
        expansion("second", 1, 50)
    );
}

// pydantic trims the value, takes a sign, lets digit groups be joined by single underscores and accepts a fraction of zeros only
#[rstest]
#[case::sign("+2", 2)]
#[case::leading_zero("010", 10)]
#[case::underscore("1_0", 10)]
#[case::zero_fraction("1.00", 1)]
#[case::surrounding_space("%201%20", 1)]
fn reads_the_pydantic_integer_spellings(#[case] text: &str, #[case] page: i64) {
    let raw = format!("session_id=s&page={text}");
    assert_eq!(decode_session_query(Some(&raw)).unwrap().page, page);
}

#[rstest]
#[case::fraction("1.5")]
#[case::empty_fraction("1.")]
#[case::no_whole(".0")]
#[case::leading_underscore("_1")]
#[case::trailing_underscore("1_")]
#[case::double_underscore("1__0")]
#[case::word("abc")]
#[case::empty("")]
#[case::exponent("1e2")]
fn refuses_text_that_is_not_an_integer(#[case] text: &str) {
    let raw = format!("session_id=s&page={text}");
    let issue = decode_session_query(Some(&raw)).unwrap_err();
    assert_eq!(issue.detail.len(), 1);
    assert_eq!(issue.detail.into_iter().next().unwrap().kind, "int_parsing");
}

#[test]
fn refuses_a_page_below_one() {
    let issue = only_issue("session_id=s&page=-0");
    assert_eq!(issue.kind, "greater_than_equal");
    assert_eq!(issue.loc, ["query", "page"]);
    assert_eq!(issue.msg, "Input should be greater than or equal to 1");
}

#[test]
fn refuses_a_page_size_above_one_hundred() {
    let issue = only_issue("session_id=s&page_size=101");
    assert_eq!(issue.kind, "less_than_equal");
    assert_eq!(issue.loc, ["query", "page_size"]);
    assert_eq!(issue.msg, "Input should be less than or equal to 100");
}

#[test]
fn refuses_a_page_size_below_one() {
    let issue = only_issue("session_id=s&page_size=0");
    assert_eq!(issue.kind, "greater_than_equal");
    assert_eq!(issue.loc, ["query", "page_size"]);
}

// the repository convention for a value below the signed 64-bit range, as B1728 recorded
#[test]
fn a_negative_page_below_the_range_is_below_the_minimum() {
    let issue = only_issue("session_id=s&page=-99999999999999999999999");
    assert_eq!(issue.kind, "greater_than_equal");
}

// B1728's deviation: LiteLLM parses an arbitrary-precision page and fails later at the database
#[test]
fn a_page_above_the_signed_range_is_int_parsing() {
    let issue = only_issue("session_id=s&page=99999999999999999999999");
    assert_eq!(issue.kind, "int_parsing");
}

// a page_size above the range still answers the upper bound, because a maximum is declared
#[test]
fn a_page_size_above_the_signed_range_is_above_the_maximum() {
    let issue = only_issue("session_id=s&page_size=99999999999999999999999");
    assert_eq!(issue.kind, "less_than_equal");
    assert_eq!(issue.msg, "Input should be less than or equal to 100");
}

#[test]
fn the_missing_session_id_answers_the_exact_framework_body() {
    let issues = decode_session_query(Some("")).unwrap_err();
    assert_eq!(
        serde_json::to_string(&issues).unwrap(),
        r#"{"detail":[{"type":"missing","loc":["query","session_id"],"msg":"Field required"}]}"#
    );
}

// FastAPI reports every bad parameter in its declared order, session_id first
#[test]
fn every_bad_parameter_is_reported_in_order() {
    let issues = decode_session_query(Some("page=0&page_size=101")).unwrap_err();
    let found: Vec<(&str, &str)> = issues
        .detail
        .iter()
        .map(|issue| (issue.kind, issue.loc[1]))
        .collect();
    assert_eq!(
        found,
        [
            ("missing", "session_id"),
            ("greater_than_equal", "page"),
            ("less_than_equal", "page_size"),
        ]
    );
}

#[test]
fn an_invalid_page_falls_back_to_the_default() {
    let issues = decode_session_query(None).unwrap_err();
    assert_eq!(
        issues,
        SessionQueryIssues {
            detail: vec![SessionQueryIssue {
                kind: "missing",
                loc: ["query", "session_id"],
                msg: String::from("Field required"),
            }],
        }
    );
}
