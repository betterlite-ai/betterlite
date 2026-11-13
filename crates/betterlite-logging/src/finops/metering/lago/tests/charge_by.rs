#![cfg(test)]

use rstest::rstest;

use super::support::{environment_with, settings_with};
use crate::finops::metering::lago::settings::{ChargeBy, LagoSettings, LagoSettingsError};

#[rstest]
#[case::unset(None, ChargeBy::EndUserId)]
#[case::empty(Some(""), ChargeBy::EndUserId)]
#[case::end_user_id(Some("end_user_id"), ChargeBy::EndUserId)]
#[case::team_id(Some("team_id"), ChargeBy::TeamId)]
#[case::user_id(Some("user_id"), ChargeBy::UserId)]
fn charge_by_selects_the_subscription_source(
    #[case] value: Option<&'static str>,
    #[case] expected: ChargeBy,
) {
    let settings = value.map_or_else(
        || settings_with(&[]),
        |charge_by_value| settings_with(&[("LAGO_API_CHARGE_BY", charge_by_value)]),
    );
    assert_eq!(settings.charge_by, expected);
}

#[rstest]
#[case::unknown("user")]
#[case::capitalized("End_User_Id")]
#[case::padded(" end_user_id ")]
#[case::other_key("account_id")]
fn charge_by_rejects_a_value_outside_the_set(#[case] value: &'static str) {
    let error = LagoSettings::from_environment(&environment_with(&[("LAGO_API_CHARGE_BY", value)]))
        .unwrap_err();
    let LagoSettingsError::InvalidChargeBy { value: reported } = error else {
        panic!("the settings error is not InvalidChargeBy");
    };
    assert_eq!(reported, value);
}
