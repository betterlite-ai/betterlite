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
    match (&decoded, &again) {
        (Ok(first), Ok(second)) => {
            assert_eq!(first.start_date, second.start_date);
            assert_eq!(first.end_date, second.end_date);
            assert_eq!(first.group_by, second.group_by);
            assert_eq!(
                first.api_key.as_ref().map(ExposeSecret::expose_secret),
                second.api_key.as_ref().map(ExposeSecret::expose_secret)
            );
            assert_eq!(first.internal_user_id, second.internal_user_id);
            assert_eq!(first.team_id, second.team_id);
            assert_eq!(first.customer_id, second.customer_id);
        }
        (Err(first), Err(second)) => assert_eq!(first, second),
        (Ok(_), Err(_)) | (Err(_), Ok(_)) => {
            panic!("two decodes of one corpus seed disagree");
        }
    }
    let carries_group_by = form_urlencoded::parse(&bytes).any(|(name, _)| name == "group_by");
    if let Ok(query) = decoded
        && !carries_group_by
    {
        assert_eq!(query.group_by, GroupBy::Team);
    }
}
