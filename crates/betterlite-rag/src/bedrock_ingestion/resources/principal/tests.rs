#![cfg(test)]

use rstest::rstest;

use crate::bedrock_ingestion::resources::principal::normalize_principal_arn;

#[rstest]
#[case::commercial(
    "arn:aws:sts::123456789012:assumed-role/MyRole/session-one",
    "123456789012",
    "arn:aws:iam::123456789012:role/MyRole"
)]
#[case::china(
    "arn:aws-cn:sts::210987654321:assumed-role/ChinaRole/session",
    "210987654321",
    "arn:aws-cn:iam::210987654321:role/ChinaRole"
)]
#[case::govcloud(
    "arn:aws-us-gov:sts::111122223333:assumed-role/GovRole/session",
    "111122223333",
    "arn:aws-us-gov:iam::111122223333:role/GovRole"
)]
#[case::second_slash_part_is_the_role(
    "arn:aws:sts::123456789012:assumed-role/path/SomeRole/session",
    "123456789012",
    "arn:aws:iam::123456789012:role/path"
)]
fn principal_rewrites_assumed_role_arns(
    #[case] caller_arn: &str,
    #[case] account_id: &str,
    #[case] expected: &str,
) {
    assert_eq!(normalize_principal_arn(caller_arn, account_id), expected);
}

#[rstest]
#[case::iam_user("arn:aws:iam::123456789012:user/alice")]
#[case::iam_role("arn:aws:iam::123456789012:role/MyRole")]
#[case::root("arn:aws:iam::123456789012:root")]
#[case::empty("")]
fn principal_keeps_other_arns(#[case] caller_arn: &str) {
    assert_eq!(
        normalize_principal_arn(caller_arn, "123456789012"),
        caller_arn
    );
}

#[rstest]
#[case::assumed_role_prefix_only("arn:aws:sts::123456789012:assumed-role")]
#[case::not_an_arn("plain-text")]
fn principal_keeps_arns_without_the_assumed_role_marker(#[case] caller_arn: &str) {
    assert_eq!(
        normalize_principal_arn(caller_arn, "123456789012"),
        caller_arn
    );
}
