#![cfg(test)]

use super::load_aws_kms;
use crate::aws_kms::{AWS_KMS_CLIENT, tests::replay};
use aws_sdk_kms::config::SharedHttpClient;
use std::{collections::BTreeMap, time::Duration};

#[test]
fn load_aws_kms_does_not_initialize_when_disabled() {
    load_aws_kms(
        Some(false),
        &BTreeMap::new(),
        SharedHttpClient::new(replay("{}", 200)),
        None,
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(AWS_KMS_CLIENT.lock().unwrap().is_none());
}

#[test]
fn load_aws_kms_builds_a_client_for_the_configured_region() {
    let environment = BTreeMap::from([("AWS_REGION_NAME".to_owned(), "us-west-2".to_owned())]);
    load_aws_kms(
        Some(true),
        &environment,
        SharedHttpClient::new(replay("{}", 200)),
        None,
        Duration::from_secs(3),
    )
    .unwrap();
    let config = AWS_KMS_CLIENT
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .config()
        .clone();
    assert_eq!(config.region().unwrap().as_ref(), "us-west-2");
    assert_eq!(
        config.timeout_config().unwrap().operation_timeout(),
        Some(Duration::from_secs(3))
    );
}
