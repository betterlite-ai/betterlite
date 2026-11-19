#![cfg(test)]

use betterlite_logging::focus_records::destinations::FocusTimeWindow;

use crate::finops::mavvrik::support::{Recorder, at, build, day, environment};

fn window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: day("2024-03-09"),
        end_time: day("2024-03-10"),
        frequency: String::from("daily"),
    }
}

#[tokio::test]
async fn the_engine_receives_the_mavvrik_defaults_and_the_destination_config() {
    let recorder = Recorder::default();
    let logger = build(
        &environment(&[
            ("MAVVRIK_API_KEY", "secret-key"),
            ("MAVVRIK_API_ENDPOINT", "https://mavvrik.example/api"),
            ("MAVVRIK_CONNECTION_ID", "connection-1"),
        ]),
        &recorder,
        None,
    );
    assert_eq!(logger.frequency, "daily");
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    let settings = recorder.settings();
    assert_eq!(settings.len(), 1);
    let settings = &settings[0];
    assert_eq!(settings.provider, "mavvrik");
    assert_eq!(settings.export_format, "csv");
    assert_eq!(settings.prefix, "mavvrik_focus_exports");
    assert_eq!(settings.api_key.as_deref(), Some("secret-key"));
    assert_eq!(
        settings.api_endpoint.as_deref(),
        Some("https://mavvrik.example/api")
    );
    assert_eq!(settings.connection_id.as_deref(), Some("connection-1"));
}

#[tokio::test]
async fn no_other_destination_setting_reaches_the_engine() {
    let recorder = Recorder::default();
    let logger = build(
        &environment(&[
            ("FOCUS_GCS_BUCKET_NAME", "a-bucket"),
            ("FOCUS_AWS_REGION_NAME", "us-east-1"),
            ("FOCUS_VANTAGE_BASE_URL", "https://vantage.example"),
        ]),
        &recorder,
        None,
    );
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    let settings = recorder.settings();
    let settings = &settings[0];
    assert_eq!(settings.bucket_name, None);
    assert_eq!(settings.region_name, None);
    assert_eq!(settings.endpoint_url, None);
    assert_eq!(settings.aws_access_key_id, None);
    assert_eq!(settings.aws_secret_access_key, None);
    assert_eq!(settings.aws_session_token, None);
    assert_eq!(settings.integration_token, None);
    assert_eq!(settings.base_url, None);
    assert_eq!(settings.service_account_json, None);
}

#[tokio::test]
async fn missing_and_empty_credentials_stay_none_for_the_destination_to_reject() {
    let recorder = Recorder::default();
    let logger = build(
        &environment(&[
            ("MAVVRIK_API_KEY", ""),
            ("MAVVRIK_API_ENDPOINT", ""),
            ("MAVVRIK_CONNECTION_ID", ""),
        ]),
        &recorder,
        None,
    );
    logger
        .export_window(&window(), None)
        .await
        .expect("the fake export succeeds");
    let settings = recorder.settings();
    let settings = &settings[0];
    assert_eq!(settings.api_key, None);
    assert_eq!(settings.api_endpoint, None);
    assert_eq!(settings.connection_id, None);
}

#[tokio::test]
async fn the_engine_is_built_once_and_reused() {
    let recorder = Recorder::default();
    let logger = build(&environment(&[]), &recorder, None);
    logger
        .export_window(&window(), None)
        .await
        .expect("the first export succeeds");
    logger
        .export_window(&window(), None)
        .await
        .expect("the second export succeeds");
    assert_eq!(recorder.builds(), 1);
    assert_eq!(recorder.settings().len(), 1);
    assert_eq!(recorder.windows().len(), 2);
    assert_eq!(recorder.windows()[0].0, at("2024-03-09T00:00:00Z"));
}
