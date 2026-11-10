#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use rstest::rstest;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use super::authority::is_same_authority_metadata_url;
use super::authorization::{
    AZURE_ENTRA_HOSTS, fetch_authorization_metadata, fetch_single_issuer_metadata,
};
use super::document::{
    DocumentError, MAX_METADATA_BYTES, ScopesValue, decode_metadata_document, document_scopes,
};
use super::fetch::{
    FetchError, FetchedDocument, MetadataFetcher, classify_status, fetch_oauth_discovery_url,
};
use super::flow::{
    DiscoveryRequest, discover_metadata, discover_recording_attempts,
    fetch_issuer_anchored_metadata,
};
use super::outcome::{DiscoveryOutcome, DiscoveryStatus};
use super::redaction::{redacted_origin_list, sanitized_error_text};
use super::resource::{ResourceMetadata, attempt_well_known_discovery, fetch_resource_metadata};
use super::settings::{DiscoverySettings, METADATA_TIMEOUT_VARIABLE};
use super::www_authenticate::parse_www_authenticate;
use crate::server_record::MCPOAuthMetadata;

include!("tests/support.rs");
include!("tests/parse.rs");
include!("tests/fetch.rs");
include!("tests/flow.rs");
