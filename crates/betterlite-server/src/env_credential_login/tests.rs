#![cfg(test)]

use betterlite_config::sections::env_credential_login::Settings;
use rstest::rstest;

use super::is_enabled;

#[rstest]
#[case::open_without_ready_sso(false, false, false, true)]
#[case::open_with_ready_sso(false, false, true, true)]
#[case::env_login_disabled_without_sso(true, false, false, false)]
#[case::env_login_disabled_with_ready_sso(true, false, true, false)]
#[case::sso_blocks_with_unready_provider(false, true, false, true)]
#[case::sso_blocks_with_ready_provider(false, true, true, false)]
#[case::both_set_without_ready_sso(true, true, false, false)]
#[case::both_set_with_ready_sso(true, true, true, false)]
fn every_flag_combination(
    #[case] env_login_disabled: bool,
    #[case] sso_blocks_password_login: bool,
    #[case] sso_provider_ready: bool,
    #[case] expected: bool,
) {
    let settings = Settings {
        env_login_disabled,
        sso_blocks_password_login,
    };
    assert_eq!(is_enabled(&settings, sso_provider_ready), expected);
}
