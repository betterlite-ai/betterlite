use betterlite_secret_managers::aws_secrets::AwsSecretClient;
use betterlite_secret_managers::aws_secrets::put_value::async_put_secret_value;
use betterlite_secret_managers::aws_secrets::replicate::async_replicate_secret;
use betterlite_secret_managers::secret_contracts::options::SecretOptions;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::Step;
use super::super::script::{Resolver, Script};
use super::write_outcome;

pub(super) async fn value_step(
    client: &AwsSecretClient<'_, Resolver, Script>,
    step: &Step,
) -> Value {
    let written = match step {
        Step::ReplicateSecret { name, regions } => {
            async_replicate_secret(client.manager, name, regions, None, None).await
        }
        Step::PutSecretValue { name, value } => {
            let secret_value = SecretString::from(value.clone());
            async_put_secret_value(
                client.manager,
                name,
                &secret_value,
                SecretOptions {
                    optional_params: None,
                    timeout: None,
                },
                client.next_token,
            )
            .await
        }
        Step::WriteSecret { .. } | Step::DeleteSecret { .. } | Step::RotateSecret { .. } => {
            return json!({"raised": true});
        }
    };
    write_outcome(written)
}
