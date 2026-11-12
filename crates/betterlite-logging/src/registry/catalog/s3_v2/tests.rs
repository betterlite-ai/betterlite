#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::registry::catalog::{CatalogError, CatalogInputs};
use crate::s3_transport::option_value::OptionValue;

use super::{build, tls_policy};

fn inputs(source: &str) -> CatalogInputs {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("the config loads");
    let callback_settings = config.sections.callback_settings.callback_settings.clone();
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    CatalogInputs {
        config: Arc::new(config),
        http: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
        env: Arc::new(BTreeMap::<String, String>::new()),
        callback_settings,
        tracker: TaskTracker::new(),
        shutdown: CancellationToken::new(),
    }
}

fn configured() -> CatalogInputs {
    inputs(
        "callback_settings:\n  s3_v2:\n    s3_bucket_name: logs\n    s3_region_name: us-east-1\n    s3_endpoint_url: http://127.0.0.1:9000\n    s3_path: team\n    s3_batch_file_upload: true\n    s3_aws_access_key_id: AKIDTEST\n    s3_aws_secret_access_key: secrettest\n",
    )
}

#[tokio::test]
async fn callback_logger_builds_the_s3_v2_logger() {
    let inputs = configured();
    let logger = build(&inputs).expect("the logger builds");
    assert_eq!(logger.callback_name(), "s3_v2");
    assert_eq!(inputs.tracker.len(), 1);
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}

#[tokio::test]
async fn callback_logger_reports_a_missing_bucket() {
    let inputs = inputs("callback_settings:\n  s3_v2:\n    s3_region_name: us-east-1\n");
    let error = build(&inputs).err().expect("the bucket is missing");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("bucket")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_unknown_setting() {
    let inputs =
        inputs("callback_settings:\n  s3_v2:\n    s3_bucket_name: logs\n    s3_bogus_setting: 1\n");
    let error = build(&inputs).err().expect("the setting is unknown");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("s3_bogus_setting")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_a_credential_flow_it_cannot_sign() {
    let inputs = inputs(
        "callback_settings:\n  s3_v2:\n    s3_bucket_name: logs\n    s3_aws_profile_name: profile\n",
    );
    let error = build(&inputs)
        .err()
        .expect("the credential flow is refused");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("s3_aws_profile_name")
    ));
}

#[tokio::test]
async fn callback_logger_refuses_an_invalid_region() {
    let inputs = inputs(
        "callback_settings:\n  s3_v2:\n    s3_bucket_name: logs\n    s3_region_name: NOT-A-REGION\n",
    );
    let error = build(&inputs).err().expect("the region is invalid");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("NOT-A-REGION")
    ));
}

#[tokio::test]
async fn callback_logger_applies_s3_verify_to_its_client() {
    let inputs = configured();
    let disabled = tls_policy(&inputs, Some(&OptionValue::Bool(false)))
        .expect("the TLS policy resolves")
        .expect("s3_verify sets a policy");
    assert_eq!(disabled.verification, Verification::Disabled);
    let enabled = tls_policy(&inputs, Some(&OptionValue::Text("true".to_owned())))
        .expect("the TLS policy resolves")
        .expect("s3_verify sets a policy");
    assert_eq!(enabled.verification, Verification::Enabled);
    assert!(
        tls_policy(&inputs, None)
            .expect("no s3_verify keeps the shared client")
            .is_none()
    );
    let error = tls_policy(&inputs, Some(&OptionValue::Text("maybe".to_owned())))
        .expect_err("s3_verify is not a bool");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("s3_verify")
    ));
}

#[tokio::test]
async fn callback_logger_builds_with_s3_verify_off() {
    let inputs = inputs(
        "callback_settings:\n  s3_v2:\n    s3_bucket_name: logs\n    s3_region_name: us-east-1\n    s3_verify: false\n    s3_aws_access_key_id: AKIDTEST\n    s3_aws_secret_access_key: secrettest\n",
    );
    let logger = build(&inputs).expect("the logger builds with its own client");
    assert_eq!(logger.callback_name(), "s3_v2");
    inputs.shutdown.cancel();
    inputs.tracker.close();
    timeout(Duration::from_secs(1), inputs.tracker.wait())
        .await
        .expect("the flush loop stops on shutdown");
}
