use std::time::Duration;

use secrecy::ExposeSecret as _;

use crate::secret_contracts::BaseSecretManager as _;
use crate::secret_contracts::options::{DeleteOptions, SecretOptions, SecretTimeout};

use super::support::{FakeTransport, Settings, actions, answer, manager, secret_client, sent_body};

#[tokio::test]
async fn delete_with_default_recovery_window() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"ARN":"arn:one","DeletionDate":1}"#)]);
    let manager = manager(&transport, Settings::default());
    let response = secret_client(&manager)
        .async_delete_secret("prod/db", DeleteOptions::default())
        .await
        .unwrap();
    assert_eq!(
        response.body.expose_secret(),
        r#"{"ARN":"arn:one","DeletionDate":1}"#
    );
    let sent = transport.sent();
    assert_eq!(actions(&sent), ["secretsmanager.DeleteSecret"]);
    let body = sent_body(&sent[0].body);
    assert_eq!(body.secret_id.as_deref(), Some("prod/db"));
    assert_eq!(body.recovery_window_in_days, Some(7));
}

#[tokio::test]
async fn delete_with_custom_recovery_window() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"ARN":"arn:one"}"#)]);
    let manager = manager(&transport, Settings::default());
    secret_client(&manager)
        .async_delete_secret(
            "prod/db",
            DeleteOptions {
                recovery_window_in_days: 30,
                operation: SecretOptions {
                    optional_params: None,
                    timeout: Some(SecretTimeout::Total(Duration::from_secs(4))),
                },
            },
        )
        .await
        .unwrap();
    let sent = transport.sent();
    assert_eq!(sent[0].timeout, Duration::from_secs(4));
    let body = sent_body(&sent[0].body);
    assert_eq!(body.recovery_window_in_days, Some(30));
}
