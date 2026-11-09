use secrecy::{ExposeSecret as _, SecretString};

use crate::aws::request_signing::AwsOptionalParams;
use crate::aws_secrets::put_value::async_put_secret_value;
use crate::secret_contracts::BaseSecretManager as _;
use crate::secret_contracts::options::{SecretOptions, SecretRotation};

use super::support::{
    FIRST_TOKEN, FakeTransport, SECOND_TOKEN, Settings, actions, answer, first_token, manager,
    second_token, secret_client, sent_body,
};

fn rotation<'borrowed>(
    current: &'borrowed str,
    new: &'borrowed str,
    value: &'borrowed SecretString,
) -> SecretRotation<'borrowed, AwsOptionalParams> {
    SecretRotation {
        current_secret_name: current,
        new_secret_name: new,
        new_secret_value: value,
        options: SecretOptions::default(),
    }
}

#[tokio::test]
async fn same_name_rotation_uses_put_value() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"ARN":"arn:put","VersionId":"v2"}"#)]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("rotated-value".to_owned());
    let response = secret_client(&manager)
        .async_rotate_secret(rotation("prod/db", "prod/db", &value))
        .await
        .unwrap();
    assert_eq!(
        response.body.expose_secret(),
        r#"{"ARN":"arn:put","VersionId":"v2"}"#
    );
    let sent = transport.sent();
    assert_eq!(actions(&sent), ["secretsmanager.PutSecretValue"]);
    let body = sent_body(&sent[0].body);
    assert_eq!(body.secret_id.as_deref(), Some("prod/db"));
    assert_eq!(
        body.name, None,
        "PutSecretValue addresses the secret by SecretId"
    );
    assert_eq!(body.secret_string.as_deref(), Some("rotated-value"));
    assert_eq!(
        body.client_request_token.as_deref(),
        Some(FIRST_TOKEN),
        "in-place rotation takes its ClientRequestToken from the injected source"
    );
}

#[tokio::test]
async fn different_name_rotation_uses_base() {
    let transport = FakeTransport::new(vec![
        answer(200, r#"{"SecretString":"old-value"}"#),
        answer(200, r#"{"ARN":"arn:new"}"#),
        answer(200, r#"{"SecretString":"new-value"}"#),
        answer(200, r#"{"ARN":"arn:old","DeletionDate":1}"#),
    ]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("new-value".to_owned());
    let response = secret_client(&manager)
        .async_rotate_secret(rotation("prod/db", "prod/new", &value))
        .await
        .unwrap();
    assert_eq!(response.body.expose_secret(), r#"{"ARN":"arn:new"}"#);
    let sent = transport.sent();
    assert_eq!(
        actions(&sent),
        [
            "secretsmanager.GetSecretValue",
            "secretsmanager.CreateSecret",
            "secretsmanager.GetSecretValue",
            "secretsmanager.DeleteSecret",
        ]
    );
    let create = sent_body(&sent[1].body);
    assert_eq!(create.name.as_deref(), Some("prod/new"));
    assert_eq!(create.secret_string.as_deref(), Some("new-value"));
    assert_eq!(create.description.as_deref(), Some("Rotated from prod/db"));
    assert_eq!(create.tags, None);
    assert_eq!(create.client_request_token.as_deref(), Some(FIRST_TOKEN));
    let delete = sent_body(&sent[3].body);
    assert_eq!(delete.secret_id.as_deref(), Some("prod/db"));
    assert_eq!(delete.recovery_window_in_days, Some(7));
}

#[tokio::test]
async fn put_secret_value_generates_token() {
    let transport = FakeTransport::new(vec![
        answer(200, r#"{"VersionId":"v2"}"#),
        answer(200, r#"{"VersionId":"v3"}"#),
    ]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("rotated-value".to_owned());
    let first = async_put_secret_value(
        &manager,
        "prod/db",
        &value,
        SecretOptions::default(),
        first_token,
    )
    .await
    .unwrap();
    let second = async_put_secret_value(
        &manager,
        "prod/db",
        &value,
        SecretOptions::default(),
        second_token,
    )
    .await
    .unwrap();
    assert_eq!(first.body.expose_secret(), r#"{"VersionId":"v2"}"#);
    assert_eq!(second.body.expose_secret(), r#"{"VersionId":"v3"}"#);
    let sent = transport.sent();
    assert_eq!(
        sent_body(&sent[0].body).client_request_token.as_deref(),
        Some(FIRST_TOKEN),
        "the first put sends its injected token"
    );
    assert_eq!(
        sent_body(&sent[1].body).client_request_token.as_deref(),
        Some(SECOND_TOKEN),
        "a different token source produces a different ClientRequestToken"
    );
}
