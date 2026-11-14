use std::io;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use decode::{ArcReply, decode_arc_challenge, decode_arc_identity_reply};
use error::ArcIdentityError;

pub mod decode;
pub mod error;
mod key_file;

const LINUX_HIMDS_PATH: &str = "/opt/azcmagent/bin/himds";
const WINDOWS_HIMDS_SUFFIX: &str = "\\AzureConnectedMachineAgent\\himds.exe";
const ARC_DEFAULT_ENDPOINT: &str = "http://localhost:40342/metadata/identity/oauth2/token";
const REFRESH_THRESHOLD_SECONDS: i64 = 7200;
const REDACTED: &str = "[redacted]";
const API_VERSION: &str = "2020-06-01";
const ARC_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArcPlatform {
    Linux,
    Windows,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcIdentity {
    SystemAssigned,
    UserAssigned,
}

pub trait ArcHost {
    fn platform(&self) -> ArcPlatform;
    fn environment(&self, name: &str) -> Option<String>;
    fn program_data(&self) -> String;
    fn program_files(&self) -> String;
    fn file_size(&self, path: &str) -> io::Result<u64>;
    fn read_text(&self, path: &str) -> io::Result<String>;
    fn now_unix_seconds(&self) -> i64;
}

#[derive(Debug, Clone)]
pub struct MetadataRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, SecretString)>,
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct MetadataResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: SecretString,
}

pub trait ArcTransport {
    fn get(&self, request: &MetadataRequest) -> io::Result<MetadataResponse>;
}

#[derive(Debug)]
pub struct ArcAccessToken {
    pub token: SecretString,
    pub expires_on: i64,
    pub token_type: String,
    pub refresh_on: Option<i64>,
}

pub fn select_arc_endpoint<H>(host: &H) -> Option<String>
where
    H: ArcHost,
{
    if let (Some(identity_endpoint), Some(_)) = (
        host.environment("IDENTITY_ENDPOINT"),
        host.environment("IMDS_ENDPOINT"),
    ) {
        return Some(identity_endpoint).filter(|endpoint| !endpoint.is_empty());
    }
    let himds_path = match host.platform() {
        ArcPlatform::Linux => LINUX_HIMDS_PATH.to_owned(),
        ArcPlatform::Windows => format!("{}{WINDOWS_HIMDS_SUFFIX}", host.program_files()),
        ArcPlatform::Other(_) => return None,
    };
    host.file_size(&himds_path)
        .is_ok()
        .then(|| ARC_DEFAULT_ENDPOINT.to_owned())
}

fn arc_request(
    endpoint: &str,
    resource: &str,
    key_secret: Option<&SecretString>,
) -> MetadataRequest {
    let mut headers = vec![("Metadata".to_owned(), SecretString::from("true"))];
    if let Some(secret) = key_secret {
        headers.push((
            "Authorization".to_owned(),
            SecretString::from(format!("Basic {}", secret.expose_secret())),
        ));
    }
    MetadataRequest {
        url: endpoint.to_owned(),
        query: vec![
            ("api-version".to_owned(), API_VERSION.to_owned()),
            ("resource".to_owned(), resource.to_owned()),
        ],
        headers,
        timeout: ARC_TIMEOUT,
    }
}

fn challenge_value(headers: &[(String, String)]) -> Result<String, ArcIdentityError> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("www-authenticate"))
        .map(|(_, value)| value.as_str())
        .collect();
    decode_arc_challenge(values.join(", ").as_bytes()).ok_or_else(|| {
        let rendered: Vec<String> = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();
        ArcIdentityError::UnrecognizableChallenge {
            headers: rendered.join(", "),
        }
    })
}

fn rejection_description(
    body: &SecretString,
    key_secret: &SecretString,
    access_token: Option<&SecretString>,
) -> String {
    let mut description = body.expose_secret().to_owned();
    let key = key_secret.expose_secret().trim();
    if !key.is_empty() {
        description = description.replace(key, REDACTED);
    }
    if let Some(token) = access_token.map(ExposeSecret::expose_secret)
        && !token.is_empty()
    {
        description = description.replace(token, REDACTED);
    }
    description
}

pub fn acquire_arc_token<H, T>(
    host: &H,
    transport: &T,
    endpoint: &str,
    resource: &str,
    identity: ArcIdentity,
) -> Result<ArcAccessToken, ArcIdentityError>
where
    H: ArcHost,
    T: ArcTransport,
{
    if identity == ArcIdentity::UserAssigned {
        return Err(ArcIdentityError::UserAssignedIdentity);
    }
    let challenge = transport
        .get(&arc_request(endpoint, resource, None))
        .map_err(ArcIdentityError::Transport)?;
    let key_name = challenge_value(&challenge.headers)?;
    let key_secret = key_file::read_key_file(host, &key_name)?;
    let reply = transport
        .get(&arc_request(endpoint, resource, Some(&key_secret)))
        .map_err(ArcIdentityError::Transport)?;
    match decode_arc_identity_reply(reply.body.expose_secret().as_bytes())? {
        ArcReply::Token(token) => {
            let now = host.now_unix_seconds();
            let refresh_on = (token.expires_in >= REFRESH_THRESHOLD_SECONDS)
                .then(|| now.saturating_add(token.expires_in.div_euclid(2)));
            Ok(ArcAccessToken {
                token: token.access_token,
                expires_on: now.saturating_add(token.expires_in),
                token_type: token.token_type,
                refresh_on,
            })
        }
        ArcReply::Rejected { access_token } => Err(ArcIdentityError::TokenRequest {
            description: rejection_description(&reply.body, &key_secret, access_token.as_ref()),
        }),
    }
}

mod tests;
