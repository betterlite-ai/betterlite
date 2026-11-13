use std::io;
use std::string::FromUtf8Error;

use thiserror::Error;

use crate::azure_arc_identity::error::ArcIdentityError;
use crate::azure_hosted_identity::error::HostedIdentityError;
use crate::oidc_exchange::OidcExchangeError;

use super::decode::ImdsTokenError;

pub const NO_ENDPOINT: &str = "No managed identity endpoint found. \nThe Target Azure platform could not be determined from environment variables. \nVisit https://aka.ms/azsdk/python/identity/managedidentitycredential/troubleshoot to troubleshoot this issue.";
pub const NO_RESPONSE_PREFIX: &str =
    "ManagedIdentityCredential authentication unavailable, no response from the IMDS endpoint.";
pub const NO_IDENTITY: &str = "ManagedIdentityCredential authentication unavailable. No identity has been assigned to this resource.";
pub const AUTHENTICATION_UNAVAILABLE: &str =
    "ManagedIdentityCredential authentication unavailable.";

#[derive(Debug, Error)]
pub enum ManagedIdentityError {
    #[error("This credential requires exactly one scope per token request.")]
    ScopeCount,
    #[error("{NO_ENDPOINT}")]
    NoEndpoint,
    #[error(
        "Workload identity was selected but no client ID was provided. Configure the environment with a client ID or pass a value for \"client_id\" argument"
    )]
    WorkloadClientId,
    #[error("{NO_RESPONSE_PREFIX} ")]
    ImdsUnavailable,
    #[error("{NO_RESPONSE_PREFIX} Token request error: ({error}) {description}")]
    TokenRequest { error: String, description: String },
    #[error("ManagedIdentityCredential authentication unavailable. Error: {message}")]
    ImdsError { message: String },
    #[error("{NO_IDENTITY}")]
    NoIdentityAssigned,
    #[error("{AUTHENTICATION_UNAVAILABLE}")]
    AuthenticationUnavailable,
    #[error("{message}")]
    ClientAuthentication { message: String },
    #[error("the Azure Arc endpoint is not available")]
    ArcEndpoint,
    #[error("the token lifetime overflows the clock")]
    ExpiryOverflow,
    #[error("the token file could not be read")]
    TokenFile {
        #[source]
        source: io::Error,
    },
    #[error("the token file is not UTF-8")]
    TokenFileEncoding {
        #[source]
        source: FromUtf8Error,
    },
    #[error(transparent)]
    ImdsReply(#[from] ImdsTokenError),
    #[error(transparent)]
    Hosted(#[from] HostedIdentityError),
    #[error(transparent)]
    Arc(#[from] ArcIdentityError),
    #[error(transparent)]
    Oidc(#[from] OidcExchangeError),
}
