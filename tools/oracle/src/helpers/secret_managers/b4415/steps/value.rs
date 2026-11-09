use betterlite_secret_managers::aws::AwsSecretsManager;
use betterlite_secret_managers::aws_secrets::put_value::async_put_secret_value;
use betterlite_secret_managers::aws_secrets::replicate::async_replicate_secret;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::Step;
use super::super::script::{Resolver, Script};
use super::write_outcome;

pub(super) async fn value_step(
    manager: &AwsSecretsManager<Resolver, Script>,
    step: &Step,
) -> Value {
    let written = match step {
        Step::AsyncReplicateSecret { name, regions } => {
            async_replicate_secret(manager, name, regions, None, None).await
        }
        Step::AsyncPutSecretValue { name, value } => {
            let secret_value = SecretString::from(value.clone());
            async_put_secret_value(manager, name, &secret_value, None, None).await
        }
        Step::AsyncWriteSecret { .. }
        | Step::AsyncDeleteSecret { .. }
        | Step::AsyncRotateSecret { .. } => {
            return json!({"raised": true});
        }
    };
    write_outcome(written)
}
