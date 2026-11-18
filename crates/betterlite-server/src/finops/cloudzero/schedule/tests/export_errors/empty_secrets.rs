use secrecy::SecretString;

use super::*;
use crate::finops::cloudzero::schedule::{
    DEFAULT_INTERVAL_MINUTES, DEFAULT_MAX_FETCHED_RECORDS, DEFAULT_TIMEZONE,
};

fn literal(api_key: &str, connection_id: &str) -> CloudZeroSettings {
    CloudZeroSettings {
        api_key: Some(SecretString::from(api_key.to_owned())),
        connection_id: Some(SecretString::from(connection_id.to_owned())),
        timezone: DEFAULT_TIMEZONE.to_owned(),
        interval_minutes: DEFAULT_INTERVAL_MINUTES,
        max_fetched_records: DEFAULT_MAX_FETCHED_RECORDS,
    }
}

#[tokio::test]
async fn cloudzero_schedule() {
    let empty_key = logger(
        literal("", "conn"),
        datastore(),
        FakeTransport::default(),
        None,
    );
    let error = export_usage_data(&empty_key, None, None, None, None)
        .await
        .expect_err("an empty API key is refused");
    assert_missing_configuration(&error);
    let empty_connection = logger(
        literal("key", ""),
        datastore(),
        FakeTransport::default(),
        None,
    );
    let error = export_usage_data(&empty_connection, None, None, None, None)
        .await
        .expect_err("an empty connection ID is refused");
    assert_missing_configuration(&error);
}
