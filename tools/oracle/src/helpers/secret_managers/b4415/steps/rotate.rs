use betterlite_secret_managers::aws_secrets::AwsSecretClient;
use betterlite_secret_managers::secret_contracts::BaseSecretManager as _;
use betterlite_secret_managers::secret_contracts::options::{SecretOptions, SecretRotation};
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::Step;
use super::super::script::{Resolver, Script};
use super::body_outcome;

pub(super) async fn rotate_step(
    client: &AwsSecretClient<'_, Resolver, Script>,
    step: &Step,
) -> Value {
    let Step::RotateSecret {
        current,
        new,
        value,
    } = step
    else {
        return json!({"raised": true});
    };
    let new_value = SecretString::from(value.clone());
    let rotated = client
        .async_rotate_secret(SecretRotation {
            current_secret_name: current,
            new_secret_name: new,
            new_secret_value: &new_value,
            options: SecretOptions {
                optional_params: None,
                timeout: None,
            },
        })
        .await;
    rotated.map_or_else(
        |_| json!({"raised": true}),
        |response| body_outcome(&response),
    )
}
