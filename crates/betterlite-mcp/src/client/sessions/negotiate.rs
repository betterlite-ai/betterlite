use std::collections::BTreeSet;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::client::SessionSettings;
use super::error::{McpFailure, SessionConfigError, SessionError};

pub const UNSUPPORTED_PROTOCOL_VERSION: i32 = -32022;
pub const CLIENT_NAME: &str = "litellm";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum LegacyVersion {
    #[serde(rename = "2024-11-05")]
    Nov2024,
    #[serde(rename = "2025-03-26")]
    Mar2025,
    #[serde(rename = "2025-06-18")]
    Jun2025,
    #[serde(rename = "2025-11-25")]
    Nov2025,
}

impl LegacyVersion {
    pub const ALL: [Self; 4] = [Self::Nov2024, Self::Mar2025, Self::Jun2025, Self::Nov2025];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nov2024 => "2024-11-05",
            Self::Mar2025 => "2025-03-26",
            Self::Jun2025 => "2025-06-18",
            Self::Nov2025 => "2025-11-25",
        }
    }
}

impl TryFrom<String> for LegacyVersion {
    type Error = SessionConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "2024-11-05" => Ok(Self::Nov2024),
            "2025-03-26" => Ok(Self::Mar2025),
            "2025-06-18" => Ok(Self::Jun2025),
            "2025-11-25" => Ok(Self::Nov2025),
            _ => Err(SessionConfigError::ProtocolVersion { value }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum ProtocolVersion {
    Auto,
    Legacy(LegacyVersion),
}

impl TryFrom<String> for ProtocolVersion {
    type Error = SessionConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "auto" {
            return Ok(Self::Auto);
        }
        LegacyVersion::try_from(value).map(Self::Legacy)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Callback {
    Sampling,
    Elicitation,
    Logging,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Callbacks {
    pub kinds: BTreeSet<Callback>,
}

impl Callbacks {
    #[must_use]
    pub fn capabilities(&self) -> ClientCapabilities {
        ClientCapabilities {
            sampling: self.kinds.contains(&Callback::Sampling),
            elicitation: self.kinds.contains(&Callback::Elicitation),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ClientCapabilities {
    pub sampling: bool,
    pub elicitation: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InitializeRequest {
    pub protocol_version: String,
    pub client_info: Implementation,
    pub capabilities: ClientCapabilities,
}

pub trait InitializeOutcome {
    fn protocol_version(&self) -> &str;

    fn instructions(&self) -> Option<&str>;
}

pub trait ClientSession {
    type Error: Error + Send + Sync + 'static;
    type InitializeResult: InitializeOutcome;

    fn initialize(
        &mut self,
    ) -> impl Future<Output = Result<Self::InitializeResult, Self::Error>> + Send;

    fn send_initialize(
        &mut self,
        request: InitializeRequest,
    ) -> impl Future<Output = Result<Self::InitializeResult, Self::Error>> + Send;

    fn adopt(&mut self, result: &Self::InitializeResult);

    fn send_initialized(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

pub async fn initialize_session<Session>(
    session: &mut Session,
    settings: &SessionSettings,
) -> Result<Session::InitializeResult, SessionError<Session::Error>>
where
    Session: ClientSession,
{
    match settings.protocol_version {
        ProtocolVersion::Auto => initialize_auto(session).await,
        ProtocolVersion::Legacy(version) => initialize_explicit(session, settings, version).await,
    }
}

async fn initialize_auto<Session>(
    session: &mut Session,
) -> Result<Session::InitializeResult, SessionError<Session::Error>>
where
    Session: ClientSession,
{
    let result = session
        .initialize()
        .await
        .map_err(|error| SessionError::Runtime(Arc::new(error)))?;
    if !LegacyVersion::ALL
        .iter()
        .any(|version| version.as_str() == result.protocol_version())
    {
        return Err(SessionError::Protocol(McpFailure {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: "Upstream selected an unsupported MCP protocol version".to_owned(),
            context: None,
        }));
    }
    Ok(result)
}

async fn initialize_explicit<Session>(
    session: &mut Session,
    settings: &SessionSettings,
    version: LegacyVersion,
) -> Result<Session::InitializeResult, SessionError<Session::Error>>
where
    Session: ClientSession,
{
    let request = InitializeRequest {
        protocol_version: version.as_str().to_owned(),
        client_info: Implementation {
            name: CLIENT_NAME.to_owned(),
            version: settings.client_version.clone(),
        },
        capabilities: settings.callbacks.capabilities(),
    };
    let result = session
        .send_initialize(request)
        .await
        .map_err(|error| SessionError::Runtime(Arc::new(error)))?;
    if result.protocol_version() != version.as_str() {
        return Err(SessionError::Protocol(McpFailure {
            code: UNSUPPORTED_PROTOCOL_VERSION,
            message: "Upstream did not accept the configured MCP protocol version".to_owned(),
            context: None,
        }));
    }
    session.adopt(&result);
    session
        .send_initialized()
        .await
        .map_err(|error| SessionError::Runtime(Arc::new(error)))?;
    Ok(result)
}
