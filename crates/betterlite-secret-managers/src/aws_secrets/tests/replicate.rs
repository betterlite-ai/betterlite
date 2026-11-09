use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_secrets::replicate::async_replicate_secret;
use crate::secret_contracts::BaseSecretManager as _;

use super::support::{
    DEFAULT_URL, FakeTransport, SentRegion, Settings, actions, answer, bodies, capture_logs,
    manager, secret_client, write_request,
};

fn replica_settings(regions: &[&str]) -> Settings {
    Settings {
        kms_key_id: None,
        replica_regions: regions.iter().map(|region| (*region).to_owned()).collect(),
    }
}

#[tokio::test]
async fn replicate_after_create_succeeds() {
    let transport = FakeTransport::new(vec![
        answer(200, r#"{"ARN":"arn:one"}"#),
        answer(200, r#"{"ARN":"arn:one","ReplicationStatus":[]}"#),
    ]);
    let manager = manager(&transport, replica_settings(&["us-west-2", "eu-west-1"]));
    let value = SecretString::from("s3cr3t".to_owned());
    let response = secret_client(&manager)
        .async_write_secret(write_request("prod/db", &value, None, None))
        .await
        .unwrap();
    assert_eq!(
        response.body.expose_secret(),
        r#"{"ARN":"arn:one"}"#,
        "the create reply is returned, not the replication reply"
    );
    let sent = transport.sent();
    assert_eq!(
        actions(&sent),
        [
            "secretsmanager.CreateSecret",
            "secretsmanager.ReplicateSecretToRegions",
        ]
    );
    assert_eq!(sent[1].url, DEFAULT_URL);
    let replicated = &bodies(&sent)[1];
    assert_eq!(replicated.secret_id.as_deref(), Some("prod/db"));
    assert_eq!(
        replicated.add_replica_regions,
        Some(vec![
            SentRegion {
                region: "us-west-2".to_owned(),
            },
            SentRegion {
                region: "eu-west-1".to_owned(),
            },
        ])
    );
}

#[tokio::test]
async fn replicate_warns_on_failure() {
    let (logs, guard) = capture_logs();
    let transport = FakeTransport::new(vec![
        answer(200, r#"{"ARN":"arn:one"}"#),
        answer(500, "replication failed"),
    ]);
    let manager = manager(&transport, replica_settings(&["us-west-2"]));
    let value = SecretString::from("s3cr3t".to_owned());
    let response = secret_client(&manager)
        .async_write_secret(write_request("prod/db", &value, None, None))
        .await
        .unwrap();
    drop(guard);
    assert_eq!(
        response.body.expose_secret(),
        r#"{"ARN":"arn:one"}"#,
        "a failed replication does not fail or roll back the created secret"
    );
    assert_eq!(
        actions(&transport.sent()),
        [
            "secretsmanager.CreateSecret",
            "secretsmanager.ReplicateSecretToRegions",
        ]
    );
    let text = logs.text();
    assert!(text.contains("WARN"), "a failed replication logs a warning");
    assert!(
        text.contains("us-west-2"),
        "the warning names the configured regions"
    );
}

#[tokio::test]
async fn replicate_empty_when_no_regions() {
    let transport = FakeTransport::new(Vec::new());
    let manager = manager(&transport, Settings::default());
    let response = async_replicate_secret(&manager, "prod/db", &[], None, None)
        .await
        .unwrap();
    assert_eq!(response.body.expose_secret(), "{}");
    assert!(transport.sent().is_empty(), "no region means no request");
}
