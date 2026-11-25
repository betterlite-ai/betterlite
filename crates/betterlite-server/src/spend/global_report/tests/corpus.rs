use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::ExposeSecret;

use crate::spend::global_report::query::{GroupBy, decode_global_report_query};

#[rstest]
fn replays_the_global_report_query_corpus(
    #[files("fuzz/corpus/global_report_query/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus seed is readable");
    let decoded = decode_global_report_query(&bytes);
    let again = decode_global_report_query(&bytes);
    assert_eq!(decoded.start_date, again.start_date);
    assert_eq!(decoded.end_date, again.end_date);
    assert_eq!(decoded.group_by, again.group_by);
    assert_eq!(
        decoded.api_key.as_ref().map(ExposeSecret::expose_secret),
        again.api_key.as_ref().map(ExposeSecret::expose_secret)
    );
    assert_eq!(decoded.internal_user_id, again.internal_user_id);
    assert_eq!(decoded.team_id, again.team_id);
    assert_eq!(decoded.customer_id, again.customer_id);
    let carries_group_by = form_urlencoded::parse(&bytes).any(|(name, _)| name == "group_by");
    if let Ok(query) = decoded
        && !carries_group_by
    {
        assert_eq!(query.group_by, GroupBy::Team);
    }
}
