use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use aws_config::timeout::TimeoutConfig;
use aws_smithy_async::rt::sleep::SharedAsyncSleep;
use aws_smithy_async::time::SharedTimeSource;
use aws_smithy_runtime_api::client::http::SharedHttpClient;
use aws_types::os_shim_internal::Fs;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::{ExposeSecret as _, SecretString};

pub mod error;
pub mod secrets;

mod selection;
mod signer;
mod sts;
mod tests;

const CREDENTIAL_PROVIDER: &str = "betterlite-rds";
const RDS_SERVICE_NAME: &str = "rds-db";
const RDS_TOKEN_LIFETIME_SECONDS: u64 = 900;
const WEB_IDENTITY_DURATION_SECONDS: i32 = 3600;

// Python's quote(safe="") leaves exactly the RFC 3986 unreserved characters unescaped
const RDS_ESCAPE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum RdsTimeout {
    // None and every Python type outside float and httpx.Timeout land here in the reference adapter
    #[default]
    SdkDefaults,
    Uniform(f64),
    ConnectRead {
        connect: Option<f64>,
        read: Option<f64>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct RdsCredentialInputs {
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub region_name: Option<String>,
    pub session_name: Option<String>,
    pub profile_name: Option<String>,
    pub role_name: Option<String>,
    pub web_identity_token: Option<SecretString>,
    pub timeout: RdsTimeout,
}

pub struct RdsSources<'source> {
    pub secrets: &'source dyn secrets::RdsSecrets,
    pub environment: &'source BTreeMap<String, String>,
    pub filesystem: Fs,
    pub client: SharedHttpClient,
    pub clock: SharedTimeSource,
    pub sleep: SharedAsyncSleep,
}

pub struct RdsEndpoint<'input> {
    pub host: &'input str,
    pub port: &'input str,
    pub user: &'input str,
}

pub struct RdsSigner {
    pub client_timeout: TimeoutConfig,
    pub session: RdsSession,
}

// the credentials stay out of Debug output and out of any panic message that prints the signer
impl fmt::Debug for RdsSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RdsSigner")
    }
}

// the resolved credentials stay behind the module boundary, so nothing can read the secret back out
pub struct RdsSession {
    credentials: aws_credential_types::Credentials,
    region: String,
    clock: SharedTimeSource,
}

pub async fn generate_iam_auth_token(
    endpoint: &RdsEndpoint<'_>,
    client: Option<&RdsSigner>,
    inputs: &RdsCredentialInputs,
    sources: RdsSources<'_>,
) -> Result<SecretString, error::RdsCredentialsError> {
    let token = if let Some(signer) = client {
        signer.db_auth_token(endpoint)?
    } else {
        let signer = RdsSigner::init(inputs.clone(), sources).await?;
        signer.db_auth_token(endpoint)?
    };
    Ok(SecretString::from(
        utf8_percent_encode(token.expose_secret(), RDS_ESCAPE).to_string(),
    ))
}

// botocore builds the RDS client config from the timeout; signing itself opens no connection
pub fn rds_timeout_config(
    timeout: RdsTimeout,
) -> Result<TimeoutConfig, error::RdsCredentialsError> {
    let seconds = |value: f64| {
        Duration::try_from_secs_f64(value)
            .map_err(|source| error::RdsCredentialsError::Timeout { source })
    };
    let mut builder = TimeoutConfig::builder();
    match timeout {
        RdsTimeout::SdkDefaults => {}
        RdsTimeout::Uniform(value) => {
            builder = builder
                .connect_timeout(seconds(value)?)
                .read_timeout(seconds(value)?);
        }
        RdsTimeout::ConnectRead { connect, read } => {
            if let Some(value) = connect {
                builder = builder.connect_timeout(seconds(value)?);
            }
            if let Some(value) = read {
                builder = builder.read_timeout(seconds(value)?);
            }
        }
    }
    Ok(builder.build())
}
