use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_contracts::BaseSecretManager as _;
use crate::secret_contracts::options::{SecretOptions, SecretWrite};

use super::support::{FakeTransport, Settings, answer, manager, region_params, secret_client};

#[test]
fn sync_read_secret_returns_the_secret() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"SecretString":"hunter2"}"#)]);
    let manager = manager(&transport, Settings::default());
    let value = secret_client(&manager)
        .sync_read_secret("prod/db", SecretOptions::default())
        .unwrap()
        .expect("the reply holds a SecretString");
    assert_eq!(value.expose_secret(), "hunter2");
}

#[tokio::test]
async fn optional_params_reach_the_request() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"ARN":"arn:one"}"#)]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let params = region_params("eu-west-1");
    let request = SecretWrite {
        secret_name: "prod/db",
        secret_value: &value,
        description: None,
        options: SecretOptions {
            optional_params: Some(&params),
            timeout: None,
        },
        tags: None,
    };
    secret_client(&manager)
        .async_write_secret(request)
        .await
        .unwrap();
    let sent = transport.sent();
    assert_eq!(
        sent[0].url, "https://secretsmanager.eu-west-1.amazonaws.com",
        "the caller's optional params reach the signed request"
    );
}
