mod rotate;
mod value;

use betterlite_secret_managers::aws_secrets::AwsSecretClient;
use betterlite_secret_managers::aws_secrets::SecretActionResponse;
use betterlite_secret_managers::secret_contracts::BaseSecretManager as _;
use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretTags, SecretWrite,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::script::{Resolver, Script};
use super::{Failure, Step};

pub(super) async fn run_step(client: &AwsSecretClient<'_, Resolver, Script>, step: &Step) -> Value {
    match step {
        Step::DeleteSecret {
            name,
            recovery_window_in_days,
        } => {
            let deleted = client
                .async_delete_secret(
                    name,
                    DeleteOptions {
                        recovery_window_in_days: recovery_window_in_days.unwrap_or(7),
                        operation: SecretOptions {
                            optional_params: None,
                            timeout: None,
                        },
                    },
                )
                .await;
            write_outcome(deleted)
        }
        Step::WriteSecret { .. } => write_step(client, step).await,
        Step::RotateSecret { .. } => rotate::rotate_step(client, step).await,
        Step::ReplicateSecret { .. } | Step::PutSecretValue { .. } => {
            value::value_step(client, step).await
        }
    }
}

async fn write_step(client: &AwsSecretClient<'_, Resolver, Script>, step: &Step) -> Value {
    let Step::WriteSecret {
        name,
        value,
        description,
        tags: requested,
    } = step
    else {
        return json!({"raised": true});
    };
    let Ok(tags) = decode_tags(requested.as_ref()) else {
        return json!({"raised": true});
    };
    let secret_value = SecretString::from(value.clone());
    let written = client
        .async_write_secret(SecretWrite {
            secret_name: name,
            secret_value: &secret_value,
            description: description.as_deref(),
            options: SecretOptions {
                optional_params: None,
                timeout: None,
            },
            tags: tags.as_ref(),
        })
        .await;
    write_outcome(written)
}

fn decode_tags(tags: Option<&Value>) -> Result<Option<SecretTags>, serde_json::Error> {
    match tags {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value::<SecretTags>(value.clone()).map(Some),
    }
}

fn write_outcome(result: Result<SecretActionResponse, Failure>) -> Value {
    let Ok(response) = result else {
        return json!({"raised": true});
    };
    body_outcome(&response)
}

pub(super) fn body_outcome(response: &SecretActionResponse) -> Value {
    let Ok(parsed) = serde_json::from_str::<Value>(response.body.expose_secret()) else {
        return Value::Null;
    };
    json!({"body": parsed})
}
