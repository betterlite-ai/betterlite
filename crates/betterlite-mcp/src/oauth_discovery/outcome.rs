use http::StatusCode;

use crate::server_record::MCPOAuthMetadata;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryStatus {
    NotFound,
    HttpError(StatusCode),
    Transport(String),
    Invalid,
    IssuerMismatch,
    CrossOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryOutcome {
    Found(MCPOAuthMetadata),
    Failed(Vec<DiscoveryStatus>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryReport {
    pub outcome: DiscoveryOutcome,
    pub attempts: Vec<String>,
}

#[must_use]
pub fn combine(
    discovered: Option<MCPOAuthMetadata>,
    preferred_scopes: Option<Vec<String>>,
    statuses: Vec<DiscoveryStatus>,
) -> DiscoveryOutcome {
    match (discovered, preferred_scopes) {
        (Some(mut metadata), Some(scopes)) => {
            metadata.scopes = Some(scopes);
            DiscoveryOutcome::Found(metadata)
        }
        (Some(metadata), None) => DiscoveryOutcome::Found(metadata),
        (None, Some(scopes)) => DiscoveryOutcome::Found(MCPOAuthMetadata {
            scopes: Some(scopes),
            authorization_url: None,
            token_url: None,
            registration_url: None,
            discovered_issuer: None,
            from_origin_fallback: false,
        }),
        (None, None) => DiscoveryOutcome::Failed(statuses),
    }
}
