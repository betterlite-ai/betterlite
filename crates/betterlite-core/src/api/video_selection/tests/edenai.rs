#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use super::error::VideoSelectionError;
use super::get_provider_video_config;

#[test]
fn edenai_reports_its_missing_video_config() {
    let error = get_provider_video_config(None, ProviderId::Edenai).unwrap_err();
    assert_eq!(
        error,
        VideoSelectionError::Unavailable {
            provider: "edenai".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "no video config is installed for 'edenai'"
    );
}
