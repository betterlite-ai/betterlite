#![cfg(test)]

use std::io::Error;
use std::sync::Mutex;

use betterlite_config::sections::settings::JsonValue;

use super::{PolicyEngine, init_policy_engine};
use crate::config_boot::error::ConfigBootError;
use crate::config_boot::test_support::{capture_logs, config_from_yaml};
use crate::job_scheduler::BoxFuture;

#[derive(Default)]
struct RecordingPolicy {
    calls: Mutex<Vec<String>>,
    fail: bool,
}

impl RecordingPolicy {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn failing() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: true,
        }
    }
}

impl PolicyEngine for RecordingPolicy {
    fn init_policies<'config>(
        &'config self,
        policies: &'config JsonValue,
        policy_attachments: Option<&'config JsonValue>,
        validate_db: bool,
    ) -> BoxFuture<'config, Result<(), ConfigBootError>> {
        let call = format!(
            "policies:{}:attachments:{}:validate:{validate_db}",
            policies.as_array().map_or(0, Vec::len),
            policy_attachments.is_some()
        );
        self.calls.lock().unwrap().push(call);
        let fail = self.fail;
        Box::pin(async move {
            if fail {
                return Err(ConfigBootError::Step {
                    step: "test policy engine",
                    source: Box::new(Error::other("the test policy engine failed")),
                });
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_missing_config_skips_the_policy_engine() {
    let ports = RecordingPolicy::default();
    let (logs, _guard) = capture_logs();
    init_policy_engine(None, true, &ports).await.unwrap();
    assert!(ports.calls().is_empty());
    assert!(
        logs.contents()
            .contains("Policy engine: config is None, skipping")
    );
}

#[tokio::test]
async fn a_config_without_policies_skips_the_policy_engine() {
    let config = config_from_yaml("model_list:\n  - model_name: gpt\n");
    let ports = RecordingPolicy::default();
    let (logs, _guard) = capture_logs();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert!(ports.calls().is_empty());
    assert!(
        logs.contents()
            .contains("Policy engine: no policies in config, skipping")
    );
}

#[tokio::test]
async fn an_empty_policy_list_is_skipped() {
    let config = config_from_yaml("policies: []\n");
    let ports = RecordingPolicy::default();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn an_empty_policy_mapping_is_skipped() {
    let config = config_from_yaml("policies: {}\n");
    let ports = RecordingPolicy::default();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn a_null_policy_key_is_skipped() {
    let config = config_from_yaml("policies:\n");
    let ports = RecordingPolicy::default();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn a_false_policy_key_is_skipped() {
    let config = config_from_yaml("policies: false\n");
    let ports = RecordingPolicy::default();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn a_zero_policy_key_is_skipped() {
    let config = config_from_yaml("policies: 0\n");
    let ports = RecordingPolicy::default();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn an_empty_string_policy_key_is_skipped() {
    let config = config_from_yaml("policies: ''\n");
    let ports = RecordingPolicy::default();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert!(ports.calls().is_empty());
}

#[tokio::test]
async fn configured_policies_reach_the_port_with_their_attachments() {
    let config = config_from_yaml("policies:\n  - name: p\npolicy_attachments:\n  - name: a\n");
    let ports = RecordingPolicy::default();
    let (logs, _guard) = capture_logs();
    init_policy_engine(Some(&config), false, &ports)
        .await
        .unwrap();
    assert_eq!(
        ports.calls(),
        vec!["policies:1:attachments:true:validate:false".to_owned()]
    );
    assert!(
        logs.contents()
            .contains("Policy engine: found 1 policies in config")
    );
}

#[tokio::test]
async fn validate_db_follows_the_database_presence() {
    let config = config_from_yaml("policies:\n  - name: p\n");
    let ports = RecordingPolicy::default();
    init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap();
    assert_eq!(
        ports.calls(),
        vec!["policies:1:attachments:false:validate:true".to_owned()]
    );
}

#[tokio::test]
async fn a_port_error_is_returned() {
    let config = config_from_yaml("policies:\n  - name: p\n");
    let ports = RecordingPolicy::failing();
    let error = init_policy_engine(Some(&config), true, &ports)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("test policy engine failed"));
}
