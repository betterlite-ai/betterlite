#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth, MCPUpstreamProtocol};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use http::{HeaderMap, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use super::json::{deserialize_json_dict, deserialize_json_list, normalize_cost_info};
use super::oauth_step::TableOAuthRequest;
use super::warnings::{
    warn_config_id_jag_server_outruns_sso, warn_legacy_delegate_auth_if_applicable,
    warn_on_server_name_fields, warn_on_shared_identifier_prefixes,
};
use super::{BuildError, BuildOptions, Decryptor, DeferredSlots, ServerBuilder, StdioPolicy};
use crate::credential_contracts::auth_config::oauth::DEFAULT_SUBJECT_TOKEN_TYPE;
use crate::discovery_slots::settings::DiscoverySettings;
use crate::oauth_discovery::fetch::{FetchError, FetchedDocument, MetadataFetcher};
use crate::server_payload::rules::MCP_STDIO_DISABLED_MESSAGE;
use crate::server_record::{MCPServer, OAuth2Flow, SecretCredential};
use crate::server_utils::prefixes::{PrefixPolicy, compute_short_server_prefix};

include!("tests/support.rs");
include!("tests/credentials.rs");
include!("tests/fields.rs");
include!("tests/oauth.rs");
include!("tests/json.rs");
include!("tests/warn.rs");
include!("tests/prefix.rs");
