use std::io;

use thiserror::Error;

const SDK_PREFIX: &str = "Azure Arc managed identity configuration not found in environment. ";

#[derive(Debug, Error)]
pub enum ArcIdentityError {
    #[error("{}Unrecognizable WWW-Authenticate header: {{{headers}}}", SDK_PREFIX)]
    UnrecognizableChallenge { headers: String },
    #[error("{}Platform {platform} was undefined and unsupported", SDK_PREFIX)]
    UnsupportedPlatform { platform: String },
    #[error("{}Local key file shall not be larger than 4KB", SDK_PREFIX)]
    KeyFileTooLarge,
    #[error(
        "{}Invalid managed_identity parameter. Azure Arc supports only system-assigned managed identity, See also https://learn.microsoft.com/en-us/azure/service-fabric/configure-existing-cluster-enable-managed-identity-token-service",
        SDK_PREFIX
    )]
    UserAssignedIdentity,
    #[error("{}Token request error: (invalid_request) {description}", SDK_PREFIX)]
    TokenRequest { description: String },
    #[error("the Azure Arc key file could not be read")]
    KeyFile(#[source] io::Error),
    #[error("the Azure Arc request failed")]
    Transport(#[source] io::Error),
    #[error("the Azure Arc token reply has an unusable value at {path}")]
    ReplyValue { path: String },
}
