#![cfg(test)]

use rstest::rstest;

mod arn;
mod boundary;
mod expiry;
mod failure;
mod fixture;
mod logs;
mod probe;

enum Scenario {
    ArnMatching,
    NoSecondAssume,
    RoleChain,
    ExpirationTtl,
    StsError,
}

#[rstest]
#[case::assumed_role_arn_matching(Scenario::ArnMatching)]
#[case::already_role_no_second_assume(Scenario::NoSecondAssume)]
#[case::different_account_role_chain(Scenario::RoleChain)]
#[case::returned_expiration_ttl(Scenario::ExpirationTtl)]
#[case::sts_error(Scenario::StsError)]
#[tokio::test]
async fn assume_role_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ArnMatching => arn::assumed_role_arn_matching(),
        Scenario::NoSecondAssume => probe::already_role_no_second_assume().await,
        Scenario::RoleChain => probe::different_account_role_chain().await,
        Scenario::ExpirationTtl => expiry::returned_expiration_ttl(),
        Scenario::StsError => failure::sts_error().await,
    }
}
