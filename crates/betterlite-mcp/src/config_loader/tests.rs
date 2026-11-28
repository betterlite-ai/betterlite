#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth, MCPTransport, MCPUpstreamProtocol};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use http::{HeaderMap, StatusCode};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::RwLock;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::config_loader::checks::config_ids_taken_by_db;
use crate::config_loader::config::McpServerConfig;
use crate::config_loader::dump::{REDACTED_VALUE, redacted_registry_dump};
use crate::config_loader::error::ConfigError;
use crate::config_loader::id::{generate_stable_server_id, pinned_config_server_id};
use crate::config_loader::loader::ConfigLoader;
use crate::config_loader::ports::{
    DcrHydrator, InstructionsCache, OpenApiToolSink, ServerDefinitionCaches,
};
use crate::credential_contracts::auth_config::oauth::DEFAULT_SUBJECT_TOKEN_TYPE;
use crate::credentials::McpAuthType;
use crate::dcr_hydration::{DcrClientStore, DcrHydration, StoreReadError};
use crate::discovery_slots::DiscoverySlots;
use crate::discovery_slots::clock::TokioClock;
use crate::discovery_slots::settings::DiscoverySettings;
use crate::oauth_discovery::fetch::{FetchError, FetchedDocument, MetadataFetcher};
use crate::openapi_tools::tool::OpenApiToolConfig;
use crate::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use crate::server_build::{Decryptor, DeferredSlots, ServerBuilder, StdioPolicy};
use crate::server_payload::rules::MCP_STDIO_DISABLED_MESSAGE;
use crate::server_record::{MCPServer, OAuth2Flow, SecretCredential};
use crate::server_utils::prefixes::{PrefixPolicy, compute_short_server_prefix};

include!("tests/support.rs");
include!("tests/config.rs");
include!("tests/id.rs");
include!("tests/checks.rs");
include!("tests/build.rs");
include!("tests/side.rs");
include!("tests/openapi.rs");
