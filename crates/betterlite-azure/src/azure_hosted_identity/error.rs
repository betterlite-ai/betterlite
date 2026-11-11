use std::io;

use thiserror::Error;

use super::msal_reply::MsalKind;

pub const SERVICE_FABRIC_ERROR_MESSAGE: &str = "Specifying a client_id or identity_config is not supported by the Service Fabric managed identity environment. The managed identity configuration is determined by the Service Fabric cluster resource configuration. See https://aka.ms/servicefabricmi for more information.";

#[derive(Debug, Error)]
pub enum HostedIdentityError {
    #[error("{prefix}Token request error: ({error}) {description}")]
    TokenRequest {
        prefix: &'static str,
        error: String,
        description: String,
    },
    #[error("{}", SERVICE_FABRIC_ERROR_MESSAGE)]
    ServiceFabricIdentityConfig,
    #[error("client_id should not be set for cloud shell managed identity.")]
    CloudShellClientId,
    #[error(
        "identity_config must not contain the following keys: object_id, resource_id, client_id"
    )]
    CloudShellIdentityConfig,
    #[error("Cloud Shell managed identity configuration not found in environment. ")]
    CloudShellUnavailable,
    #[error("No token received.")]
    NoToken,
    #[error("Unexpected response \"{content}\"")]
    UnexpectedResponse { content: String },
    #[error("Failed to deserialize JSON from response")]
    JsonContentType(#[source] serde_json::Error),
    #[error("Unexpected content type \"{content_type}\"")]
    UnsupportedContentType {
        content_type: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the token reply is missing {field}")]
    MissingField { field: &'static str },
    #[error("the token reply has an unusable value at {field}")]
    ReplyValue { field: String },
    #[error("the token reply is not JSON")]
    ReplyJson(#[source] serde_json::Error),
    #[error("the {protocol:?} environment settings are not present")]
    MissingEnvironment { protocol: MsalKind },
    #[error("the hosted identity request failed")]
    Transport(#[source] io::Error),
    #[error("the token lifetime overflows the clock")]
    ExpiryOverflow,
}
