#![cfg(test)]

use super::load_aws_kms;
use crate::aws_kms::{AWS_KMS_CLIENT, tests::replay};
use aws_sdk_kms::config::SharedHttpClient;
use std::{collections::BTreeMap, time::Duration};

#[test]
fn client_storage_replaces_a_previously_loaded_client() {
    let mut environment = BTreeMap::from([("AWS_REGION_NAME".to_owned(), "us-east-1".to_owned())]);
    load_aws_kms(
        Some(true),
        &environment,
        SharedHttpClient::new(replay("{}", 200)),
        None,
        Duration::from_secs(1),
    )
    .unwrap();
    environment.insert("AWS_REGION_NAME".to_owned(), "us-west-2".to_owned());
    load_aws_kms(
        Some(true),
        &environment,
        SharedHttpClient::new(replay("{}", 200)),
        None,
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        AWS_KMS_CLIENT
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .config()
            .region()
            .unwrap()
            .as_ref(),
        "us-west-2"
    );
}
