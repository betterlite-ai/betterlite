use std::convert::Infallible;
use std::io;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::aws::request_signing::AwsRequestError;
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::BaseSecretManager as _;
use crate::secret_contracts::options::{SecretTag, SecretTags, SecretTimeout};

use super::support::{
    FIRST_TOKEN, FakeTransport, Reply, SentTag, Settings, actions, answer, bodies, manager,
    secret_client, timed_out, write_request,
};

const CREATE_COLLISION: &str = "ResourceExistsException: the secret is scheduled for deletion";
const DELETED_DATE: &str = r#"{"ARN":"arn:one","DeletedDate":1700000000}"#;
const RESTORE_ACTIONS: [&str; 4] = [
    "secretsmanager.CreateSecret",
    "secretsmanager.DescribeSecret",
    "secretsmanager.RestoreSecret",
    "secretsmanager.UpdateSecret",
];

fn collision_replies() -> Vec<Reply> {
    vec![
        answer(409, CREATE_COLLISION),
        answer(200, DELETED_DATE),
        answer(200, r#"{"ARN":"arn:restored"}"#),
    ]
}

fn tag() -> SecretTags {
    SecretTags::List(vec![SecretTag {
        key: "Environment".to_owned(),
        value: "Prod".to_owned(),
    }])
}

#[tokio::test]
async fn restore_on_deletion_scheduled() {
    let mut replies = collision_replies();
    replies.push(answer(200, r#"{"ARN":"arn:updated","VersionId":"v2"}"#));
    let transport = FakeTransport::new(replies);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let response = secret_client(&manager)
        .async_write_secret(write_request("prod/db", &value, None, None))
        .await
        .unwrap();
    assert_eq!(
        response.body.expose_secret(),
        r#"{"ARN":"arn:updated","VersionId":"v2"}"#
    );
    let sent = transport.sent();
    assert_eq!(actions(&sent), RESTORE_ACTIONS);
    let bodies = bodies(&sent);
    assert_eq!(
        bodies[1..4]
            .iter()
            .map(|restored| restored.secret_id.as_deref())
            .collect::<Vec<_>>(),
        [Some("prod/db"), Some("prod/db"), Some("prod/db")]
    );
    assert_eq!(bodies[3].name, None);
    assert_eq!(bodies[3].secret_string.as_deref(), Some("s3cr3t"));
    assert_eq!(bodies[3].tags, None);
    assert_eq!(bodies[3].description, None);
    assert_eq!(bodies[3].kms_key_id, None);
    assert_eq!(
        bodies[3].client_request_token, bodies[0].client_request_token,
        "UpdateSecret reuses the create request token"
    );
    assert_eq!(
        bodies[0].client_request_token.as_deref(),
        Some(FIRST_TOKEN),
        "the created token comes from the injected source"
    );
}

#[tokio::test]
async fn restore_with_separate_tags() {
    let mut replies = collision_replies();
    replies.push(answer(200, r#"{"ARN":"arn:updated"}"#));
    replies.push(answer(200, "{}"));
    let transport = FakeTransport::new(replies);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let tags = tag();
    let response = secret_client(&manager)
        .async_write_secret(write_request("prod/db", &value, Some(&tags), None))
        .await
        .unwrap();
    assert_eq!(response.body.expose_secret(), r#"{"ARN":"arn:updated"}"#);
    let sent = transport.sent();
    let mut expected = RESTORE_ACTIONS.to_vec();
    expected.push("secretsmanager.TagResource");
    assert_eq!(actions(&sent), expected);
    let bodies = bodies(&sent);
    assert_eq!(bodies[3].tags, None, "UpdateSecret never carries Tags");
    assert_eq!(bodies[4].secret_id.as_deref(), Some("prod/db"));
    assert_eq!(
        bodies[4].tags,
        Some(vec![SentTag {
            key: "Environment".to_owned(),
            value: "Prod".to_owned(),
        }])
    );
}

#[tokio::test]
async fn update_tag_re_deletion_on_tag_error() {
    let mut replies = collision_replies();
    replies.push(answer(200, r#"{"ARN":"arn:updated"}"#));
    replies.push(answer(400, "tag denied"));
    replies.push(answer(200, r#"{"ARN":"arn:deleted"}"#));
    let transport = FakeTransport::new(replies);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let tags = tag();
    let error = secret_client(&manager)
        .async_write_secret(write_request("prod/db", &value, Some(&tags), None))
        .await
        .unwrap_err();
    assert_http_error(&error, "tag denied");
    let sent = transport.sent();
    let mut expected = RESTORE_ACTIONS.to_vec();
    expected.push("secretsmanager.TagResource");
    expected.push("secretsmanager.DeleteSecret");
    assert_eq!(actions(&sent), expected);
    let deleted = &bodies(&sent)[5];
    assert_eq!(deleted.secret_id.as_deref(), Some("prod/db"));
    assert_eq!(deleted.recovery_window_in_days, Some(7));
}

fn assert_http_error(error: &AwsReadError<Infallible, io::Error>, text: &str) {
    match error {
        AwsReadError::Request(AwsRequestError::Http { text: sent }) => assert_eq!(sent, text),
        other @ (AwsReadError::Request(_) | AwsReadError::Primary(_)) => {
            panic!("expected an HTTP status error, got {other:?}")
        }
    }
}

#[tokio::test]
async fn http_error_raises_value_error() {
    let transport = FakeTransport::new(vec![answer(409, CREATE_COLLISION), answer(200, "{}")]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let error = secret_client(&manager)
        .async_write_secret(write_request("prod/db", &value, None, None))
        .await
        .unwrap_err();
    assert_http_error(&error, CREATE_COLLISION);
    assert_eq!(
        actions(&transport.sent()),
        [
            "secretsmanager.CreateSecret",
            "secretsmanager.DescribeSecret",
        ],
        "a raised create is checked for a pending deletion and then re-raised"
    );
}

#[tokio::test]
async fn timeout_propagates_without_a_restore_check() {
    let transport = FakeTransport::new(vec![timed_out()]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let timeout = Some(SecretTimeout::Total(Duration::from_secs(1)));
    let error = secret_client(&manager)
        .async_write_secret(write_request("prod/db", &value, None, timeout))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AwsReadError::Request(AwsRequestError::Timeout)),
        "a timeout is litellm.Timeout, which the recovery paths do not catch"
    );
    assert_eq!(
        actions(&transport.sent()),
        ["secretsmanager.CreateSecret"],
        "a timeout sends no DescribeSecret"
    );
}
