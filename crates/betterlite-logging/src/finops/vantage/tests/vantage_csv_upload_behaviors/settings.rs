use rstest::rstest;
use secrecy::SecretString;

use crate::finops::vantage::destination::VantageDestination;
use crate::finops::vantage::settings::{SettingsError, VantageSettings};
use crate::finops::vantage::tests::support::FakeSender;

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

#[rstest]
#[case::missing_api_key(None, Some(secret("token")), true)]
#[case::empty_api_key(Some(secret("")), Some(secret("token")), true)]
#[case::missing_integration_token(Some(secret("key")), None, false)]
#[case::empty_integration_token(Some(secret("key")), Some(secret("")), false)]
#[case::both_missing_reports_the_api_key_first(None, None, true)]
fn missing_credentials_are_refused_at_construction(
    #[case] api_key: Option<SecretString>,
    #[case] integration_token: Option<SecretString>,
    #[case] api_key_is_named: bool,
) {
    let settings = VantageSettings {
        api_key,
        integration_token,
        base_url: None,
    };
    let Err(error) = VantageDestination::new(FakeSender::default(), settings) else {
        panic!("settings without a usable credential were accepted");
    };
    let named = if api_key_is_named {
        matches!(error, SettingsError::MissingApiKey)
    } else {
        matches!(error, SettingsError::MissingIntegrationToken)
    };
    assert!(named, "{error}");
}
