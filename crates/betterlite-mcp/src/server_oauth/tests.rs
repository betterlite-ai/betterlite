#![cfg(test)]

use std::borrow::Cow;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth};
use rstest::rstest;
use secrecy::SecretString;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use super::basics::{
    UPSTREAM_DISCOVERY_AUTH_TYPES, blank_to_none, has_discovery_source, obo_needs_discovery,
    uses_issuer_anchor,
};
use super::flow::{
    FlowResolution, effective_oauth2_flow, explicit_oauth2_flow, flow_for_request,
    resolve_oauth2_flow,
};
use super::issuer::{
    ManualEndpoints, authorize_endpoint_for_log, endpoints_yield_to_issuer, issuer_matches,
    normalized_authorize_endpoint,
};
use super::merge::{ScopesInput, extract_scopes, merge_discovered};
use super::trust::{carry_forward_endpoints, endpoints_corroborate, restrict_to_corroborated};
use super::unresolved::{endpoints_unresolved, flow_endpoints_missing, requires_discovery};
use super::warnings::{
    NeededEndpoint, UnresolvedCheck, discovery_leaves_unresolved, warn_unresolved,
};
use crate::server_record::{MCPOAuthMetadata, MCPServer, OAuth2Flow};

include!("tests/basics.rs");
include!("tests/unresolved.rs");
include!("tests/trust.rs");
include!("tests/flow.rs");
include!("tests/merge.rs");

fn server_from_json(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

fn oauth_server() -> MCPServer {
    server_from_json(
        r#"{
            "server_id": "test-server",
            "name": "test-server",
            "url": "https://mcp.test/mcp",
            "transport": "http",
            "auth_type": "oauth2"
        }"#,
    )
}

fn metadata_from_json(json: &str) -> MCPOAuthMetadata {
    serde_json::from_str(json).unwrap()
}

fn discovered_metadata() -> MCPOAuthMetadata {
    metadata_from_json(
        r#"{
            "scopes": ["read"],
            "authorization_url": "https://other.test/authorize",
            "token_url": "https://other.test/token",
            "registration_url": "https://other.test/register",
            "discovered_issuer": "https://other.test"
        }"#,
    )
}

fn unresolved_check() -> UnresolvedCheck<'static> {
    UnresolvedCheck {
        server_ref: "test-server",
        server_url: Some("https://mcp.test:8443/mcp?token=secret"),
        discovery_attempted: true,
        discovery_deferred: false,
        issuer_anchored: false,
        metadata: None,
        needs_authorization_url: true,
        needs_token_url: true,
        manual_authorization_url: None,
        manual_token_url: None,
    }
}

fn json_string(text: &str) -> JsonValue {
    JsonValue::String(text.to_owned())
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
