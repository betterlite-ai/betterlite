#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock as StdRwLock};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_prompts::arguments::json::Object;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use http::{HeaderMap, StatusCode};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::RwLock as AsyncRwLock;
use tokio_util::task::TaskTracker;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use super::error::MutationError;
use super::ports::{
    CacheInvalidator, ServerRows, SharedToolOwners, ToolListError, ToolLister, ToolRegistrySink,
};
use super::{RegistryMutator, WarnedIds};
use crate::config_loader::ports::{DcrHydrator, InstructionsCache, OpenApiToolSink};
use crate::discovery_slots::DiscoverySlots;
use crate::discovery_slots::clock::TokioClock;
use crate::discovery_slots::settings::DiscoverySettings;
use crate::oauth_discovery::fetch::{FetchError, FetchedDocument, MetadataFetcher};
use crate::openapi_tools::tool::OpenApiToolConfig;
use crate::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use crate::registry::tool_owner::ToolOwners;
use crate::server_build::{Decryptor, DeferredSlots, ServerBuilder, StdioPolicy};
use crate::server_record::MCPServer;
use crate::server_utils::prefixes::{PrefixPolicy, compute_short_server_prefix};
use crate::tools::schemas::McpTool;
use crate::wire_compat::errors::UpstreamAuthError;

include!("tests/support.rs");
include!("tests/stage.rs");
include!("tests/swap.rs");
include!("tests/warn.rs");
include!("tests/mutation.rs");
include!("tests/cleanup.rs");
