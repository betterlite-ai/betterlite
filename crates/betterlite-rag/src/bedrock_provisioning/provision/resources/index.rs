use std::str;
use std::time::Duration;

use betterlite_context::aws_partition::get_aws_partition;
use tokio::time::sleep;
use url::Url;

use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_ingestion::error::{BedrockIngestionError, aws_error_body};
use crate::bedrock_provisioning::calls::CallRequest;
use crate::bedrock_provisioning::calls::aoss;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

pub(super) const INDEX_OPERATION: &str = "CreateIndex";
const INDEX_ATTEMPTS: usize = 8;
const INDEX_RETRY_DELAY: Duration = Duration::from_secs(20);
const INDEX_NAME: &str = "bedrock-kb-index";
const AUTHORIZATION_MARKERS: [&str; 2] = ["authorization_exception", "security_exception"];

pub(super) async fn create_index<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    region: &str,
    url: &str,
    body: Vec<u8>,
) -> Result<(), ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let mut last_status = 0_u16;
    let mut last_body = Vec::new();
    for _ in 0..INDEX_ATTEMPTS {
        let request = index_request(region, url, body.clone());
        let response = provisioner.send(&request).await?;
        if (200..300).contains(&response.status) {
            return Ok(());
        }
        if !is_authorization_failure(&response.body) {
            let failure = BedrockIngestionError::Aws {
                operation: INDEX_OPERATION,
                status: response.status,
                body: aws_error_body(&response.body),
            };
            return Err(ProvisionError::from(failure));
        }
        last_status = response.status;
        last_body = response.body;
        sleep(INDEX_RETRY_DELAY).await;
    }
    Err(ProvisionError::IndexRetriesExhausted {
        attempts: INDEX_ATTEMPTS,
        status: last_status,
        body: aws_error_body(&last_body),
    })
}

pub(super) fn index_url(endpoint: &str, region: &str) -> Result<String, CallError> {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    let parsed = Url::parse(endpoint).map_err(|_source| CallError::CollectionEndpoint {
        endpoint: endpoint.to_owned(),
    })?;
    let on_aoss = parsed
        .host_str()
        .is_some_and(|host| host.ends_with(&format!(".aoss.{suffix}")));
    if parsed.scheme() != "https" || !on_aoss {
        return Err(CallError::CollectionEndpoint {
            endpoint: endpoint.to_owned(),
        });
    }
    Ok(format!("{}/{}", endpoint.trim_end_matches('/'), INDEX_NAME))
}

fn index_request(region: &str, url: &str, body: Vec<u8>) -> CallRequest {
    CallRequest {
        operation: INDEX_OPERATION,
        service: aoss::AOSS_SERVICE,
        signing_region: region.to_owned(),
        method: "PUT",
        url: url.to_owned(),
        headers: vec![(
            "Content-Type".to_owned(),
            aoss::JSON_CONTENT_TYPE.to_owned(),
        )],
        body,
    }
}

fn is_authorization_failure(body: &[u8]) -> bool {
    let Ok(text) = str::from_utf8(body) else {
        return false;
    };
    let lowered = text.to_lowercase();
    AUTHORIZATION_MARKERS
        .iter()
        .any(|marker| lowered.contains(*marker))
}
