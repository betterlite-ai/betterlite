use betterlite_db::mcp_stored_contracts::MCPTransport;
use serde::{Deserialize, Serialize};

pub const GATEWAY_OPERATIONS: [&str; 7] = [
    "tools/list",
    "tools/call",
    "prompts/list",
    "prompts/get",
    "resources/list",
    "resources/read",
    "resources/templates/list",
];

pub const HANDSHAKE_VERSIONS: [SpecVersion; 4] = [
    SpecVersion::Nov2024,
    SpecVersion::Mar2025,
    SpecVersion::Jun2025,
    SpecVersion::Nov2025,
];

pub const LATEST_HANDSHAKE_VERSION: &str = "2025-11-25";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpecVersion {
    Nov2024,
    Mar2025,
    Jun2025,
    Nov2025,
    Jul2026,
}

impl SpecVersion {
    pub const ALL: [Self; 5] = [
        Self::Nov2024,
        Self::Mar2025,
        Self::Jun2025,
        Self::Nov2025,
        Self::Jul2026,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nov2024 => "2024-11-05",
            Self::Mar2025 => "2025-03-26",
            Self::Jun2025 => "2025-06-18",
            Self::Nov2025 => "2025-11-25",
            Self::Jul2026 => "2026-07-28",
        }
    }

    #[must_use]
    pub fn from_wire(version: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|known| known.as_str() == version)
    }

    #[must_use]
    pub const fn is_handshake(self) -> bool {
        match self {
            Self::Nov2024 | Self::Mar2025 | Self::Jun2025 | Self::Nov2025 => true,
            Self::Jul2026 => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultKind {
    Complete,
    InputRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionSupport {
    pub transports: &'static [MCPTransport],
    pub operations: &'static [&'static str],
    pub results: &'static [ResultKind],
    pub extensions: &'static [&'static str],
    pub completed: bool,
}

#[must_use]
pub const fn revision_support(version: SpecVersion) -> RevisionSupport {
    match version {
        SpecVersion::Jul2026 => RevisionSupport {
            transports: &[MCPTransport::Http, MCPTransport::Stdio],
            operations: &GATEWAY_OPERATIONS,
            results: &[ResultKind::Complete, ResultKind::InputRequired],
            extensions: &[],
            completed: false,
        },
        SpecVersion::Nov2024
        | SpecVersion::Mar2025
        | SpecVersion::Jun2025
        | SpecVersion::Nov2025 => RevisionSupport {
            transports: &[MCPTransport::Sse, MCPTransport::Http, MCPTransport::Stdio],
            operations: &GATEWAY_OPERATIONS,
            results: &[ResultKind::Complete],
            extensions: &[],
            completed: true,
        },
    }
}

mod tests;
