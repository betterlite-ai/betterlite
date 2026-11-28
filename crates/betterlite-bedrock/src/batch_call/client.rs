use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::aws_settings::pop_aws_auth_params;
use crate::aws_settings::sources::Credentials;
use crate::batch_status::handler::SIGNATURE_VERSION;
use crate::credentials::CredentialSource;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::auth_settings;

use super::error::BatchCallError;
use super::transport::{
    BEDROCK_SERVICE, ClientConfig, ControlPlaneFactory, ControlPlaneReply, ControlPlaneRequest,
    ControlPlaneTransport,
};

// the boto3 bedrock client both handlers build from their resolved credentials
pub struct ControlPlane<Client> {
    pub credentials: Credentials,
    pub transport: Client,
}

// boto3.client("bedrock", ...) with the resolved credentials and Config(signature_version="v4")
pub async fn control_plane<Source, Factory>(
    kwargs: &BTreeMap<String, Box<RawValue>>,
    region: &str,
    source: &Source,
    factory: &Factory,
) -> Result<ControlPlane<Factory::Client>, BatchCallError>
where
    Source: CredentialSource,
    Factory: ControlPlaneFactory,
{
    let mut parameters = kwargs.clone();
    let auth = pop_aws_auth_params(&mut parameters)?;
    let settings = auth_settings(auth, region);
    let credentials = source.resolve(settings, region.to_owned()).await?;
    let transport = factory.client(ClientConfig {
        service_name: BEDROCK_SERVICE,
        region,
        credentials: &credentials,
        signature_version: SIGNATURE_VERSION,
    });
    Ok(ControlPlane {
        credentials,
        transport,
    })
}

pub async fn send<Client>(
    transport: Client,
    signed: AwsSignedRequest,
) -> Result<ControlPlaneReply, BatchCallError>
where
    Client: ControlPlaneTransport,
{
    let request = ControlPlaneRequest {
        method: signed.method,
        url: signed.url,
        headers: signed.headers,
        body: signed.body,
    };
    transport.send(request).await.map_err(BatchCallError::from)
}
