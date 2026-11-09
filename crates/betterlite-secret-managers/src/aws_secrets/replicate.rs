use secrecy::SecretString;

use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsTransport, CredentialResolver,
};
use crate::aws_secret_reads::AwsReadError;
use crate::secret_contracts::options::SecretTimeout;

use super::SecretActionResponse;
use super::bodies::{ReplicaRegion, ReplicateSecretBody};

const REPLICATE_SECRET: &str = "ReplicateSecretToRegions";

pub async fn async_replicate_secret<Resolver, Transport>(
    manager: &AwsSecretsManager<Resolver, Transport>,
    secret_name: &str,
    replica_regions: &[String],
    optional_params: Option<&mut AwsOptionalParams>,
    timeout: Option<SecretTimeout>,
) -> Result<SecretActionResponse, AwsReadError<Resolver::Error, Transport::Error>>
where
    Resolver: CredentialResolver + Sync,
    Transport: AwsTransport,
{
    if replica_regions.is_empty() {
        return Ok(SecretActionResponse {
            body: SecretString::from("{}".to_owned()),
        });
    }
    let body = ReplicateSecretBody {
        secret_id: secret_name,
        add_replica_regions: replica_regions
            .iter()
            .map(|region| ReplicaRegion {
                region: region.as_str(),
            })
            .collect(),
    };
    let request = ActionRequest {
        action: REPLICATE_SECRET,
        secret_name,
        request_data: Some(&body),
    };
    super::post_reply(manager, &request, optional_params, timeout).await
}
