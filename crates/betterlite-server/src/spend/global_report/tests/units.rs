use rstest::rstest;
use secrecy::ExposeSecret;

use crate::spend::global_report::query::{
    GlobalReportQuery, GroupBy, InvalidGroupBy, decode_global_report_query,
};

fn decoded(raw: &str) -> GlobalReportQuery {
    decode_global_report_query(raw.as_bytes()).unwrap()
}

#[rstest]
#[case::absent_defaults_to_team("start_date=2024-01-01", Ok(GroupBy::Team))]
#[case::team("group_by=team", Ok(GroupBy::Team))]
#[case::customer("group_by=customer", Ok(GroupBy::Customer))]
#[case::api_key("group_by=api_key", Ok(GroupBy::ApiKey))]
#[case::the_last_value_wins("group_by=customer&group_by=api_key", Ok(GroupBy::ApiKey))]
#[case::empty("group_by=", Err(InvalidGroupBy))]
#[case::bare_name("group_by", Err(InvalidGroupBy))]
#[case::null_text("group_by=null", Err(InvalidGroupBy))]
#[case::other_case("group_by=API_KEY", Err(InvalidGroupBy))]
fn group_by_takes_the_three_literals_or_the_team_default(
    #[case] raw: &str,
    #[case] expected: Result<GroupBy, InvalidGroupBy>,
) {
    let group_by = decode_global_report_query(raw.as_bytes()).map(|query| query.group_by);
    assert_eq!(group_by, expected);
}

#[test]
fn every_parameter_lands_in_its_own_field() {
    let query = decoded(
        "start_date=2024-01-01&end_date=2024-01-31&api_key=k&internal_user_id=u&team_id=t&customer_id=c",
    );
    assert_eq!(query.start_date.as_deref(), Some("2024-01-01"));
    assert_eq!(query.end_date.as_deref(), Some("2024-01-31"));
    assert_eq!(
        query.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("k")
    );
    assert_eq!(query.internal_user_id.as_deref(), Some("u"));
    assert_eq!(query.team_id.as_deref(), Some("t"));
    assert_eq!(query.customer_id.as_deref(), Some("c"));
}

#[test]
fn absent_parameters_stay_absent() {
    let query = decoded("unrelated=1");
    assert!(query.start_date.is_none());
    assert!(query.end_date.is_none());
    assert!(query.api_key.is_none());
    assert!(query.internal_user_id.is_none());
    assert!(query.team_id.is_none());
    assert!(query.customer_id.is_none());
}

#[test]
fn empty_parameters_are_present() {
    let query = decoded("api_key=&internal_user_id&team_id=&customer_id=");
    assert_eq!(
        query.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("")
    );
    assert_eq!(query.internal_user_id.as_deref(), Some(""));
    assert_eq!(query.team_id.as_deref(), Some(""));
    assert_eq!(query.customer_id.as_deref(), Some(""));
}

#[test]
fn a_repeated_parameter_keeps_its_last_value() {
    let query = decoded("team_id=a&customer_id=x&team_id=b&start_date=bad&start_date=2024-01-01");
    assert_eq!(query.team_id.as_deref(), Some("b"));
    assert_eq!(query.customer_id.as_deref(), Some("x"));
    assert_eq!(query.start_date.as_deref(), Some("2024-01-01"));
}

#[test]
fn values_are_percent_decoded() {
    let query = decoded("api_key=sk%2Dkey+one&customer_id=c%26d");
    assert_eq!(
        query.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-key one")
    );
    assert_eq!(query.customer_id.as_deref(), Some("c&d"));
}
