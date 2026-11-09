use secrecy::ExposeSecret as _;
use uuid::Uuid;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{AwsOptionalParams, AwsTransport, CredentialResolver};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::options::SecretWrite;

use super::bodies::{CreateSecretBody, normalized_tags};
use super::restore::create_or_restore;
use super::{SecretActionResponse, replicate};

pub(super) async fn write<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    write: SecretWrite<'_, AwsOptionalParams>,
    next_token: fn() -> Uuid,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    let token = next_token().to_string();
    let tags = write.tags.map(normalized_tags);
    let body = CreateSecretBody {
        name: write.secret_name,
        secret_string: write.secret_value.expose_secret(),
        client_request_token: &token,
        description: write.description,
        kms_key_id: manager.kms_key_id.as_deref(),
        tags: tags.as_deref(),
    };
    let mut optional_params = write.options.optional_params.map(super::copy_params);
    let timeout = write.options.timeout;
    let response = create_or_restore(manager, &body, optional_params.as_mut(), timeout).await?;
    if manager.replica_regions.is_empty() {
        return Ok(response);
    }
    let replication = replicate::async_replicate_secret(
        manager,
        write.secret_name,
        &manager.replica_regions,
        optional_params.as_mut(),
        timeout,
    )
    .await;
    if replication.is_err() {
        tracing::warn!(
            secret = write.secret_name,
            regions = ?manager.replica_regions,
            "the secret was created but replication to the configured regions failed"
        );
    }
    Ok(response)
}
