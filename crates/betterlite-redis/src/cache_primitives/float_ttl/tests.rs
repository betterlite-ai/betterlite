#![cfg(test)]

use rstest::rstest;

use super::{TtlFollowup, needs_standalone_expire, plan_ttl_followup};

#[rstest]
#[case::no_ttl_sends_no_followup(None, false, None)]
#[case::no_ttl_ignores_refresh_flag(None, true, None)]
#[case::ttl_without_refresh_reads_ttl(Some(30), false, Some(TtlFollowup::Ttl))]
#[case::ttl_with_refresh_sends_expire(Some(30), true, Some(TtlFollowup::Expire))]
fn plans_the_pipelined_followup(
    #[case] ttl: Option<i64>,
    #[case] refresh_ttl: bool,
    #[case] expected: Option<TtlFollowup>,
) {
    assert_eq!(plan_ttl_followup(ttl, refresh_ttl), expected);
}

#[rstest]
#[case::unexpiring_key_needs_expire(Some(TtlFollowup::Ttl), -1, true)]
#[case::expiring_key_needs_nothing(Some(TtlFollowup::Ttl), 30, false)]
#[case::expire_followup_never_needs_another_expire(Some(TtlFollowup::Expire), -1, false)]
#[case::no_followup_never_needs_an_expire(None, -1, false)]
fn decides_whether_a_standalone_expire_is_needed(
    #[case] followup: Option<TtlFollowup>,
    #[case] ttl_reply: i64,
    #[case] expected: bool,
) {
    assert_eq!(needs_standalone_expire(followup, ttl_reply), expected);
}
