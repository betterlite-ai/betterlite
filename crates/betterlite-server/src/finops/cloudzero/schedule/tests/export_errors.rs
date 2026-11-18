#![cfg(test)]

use std::collections::BTreeMap;

use super::fixtures::{FakeTransport, Refused, datastore, logger};
use crate::finops::cloudzero::schedule::CloudZeroSettings;
use crate::finops::cloudzero::schedule::error::CloudZeroExportError;
use crate::finops::cloudzero::schedule::export::export_usage_data;

const MESSAGE: &str = "CloudZero configuration missing. Please set CLOUDZERO_API_KEY and CLOUDZERO_CONNECTION_ID environment variables.";

async fn export_error(
    api_key: Option<&str>,
    connection_id: Option<&str>,
) -> CloudZeroExportError<Refused> {
    let empty: BTreeMap<String, String> = BTreeMap::new();
    let settings = CloudZeroSettings::resolve(api_key, connection_id, None, &empty)
        .expect("resolved settings");
    let assembled = logger(settings, datastore(), FakeTransport::default(), None);
    export_usage_data(&assembled, None, None, None, None)
        .await
        .expect_err("no database is reachable")
}

fn assert_missing_configuration(error: &CloudZeroExportError<Refused>) {
    assert!(matches!(error, CloudZeroExportError::MissingConfiguration));
    assert_eq!(error.to_string(), MESSAGE);
}

#[tokio::test]
async fn cloudzero_schedule() {
    assert_missing_configuration(&export_error(None, None).await);
    assert_missing_configuration(&export_error(None, Some("conn")).await);
    assert_missing_configuration(&export_error(Some("key"), None).await);
    assert_missing_configuration(&export_error(Some(""), Some("conn")).await);
    assert_missing_configuration(&export_error(Some("key"), Some("")).await);
    assert!(matches!(
        export_error(Some("key"), Some("conn")).await,
        CloudZeroExportError::Usage(_)
    ));
}

mod empty_secrets;
