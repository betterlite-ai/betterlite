#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, pending};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::{HeaderMap, StatusCode};
use rstest::rstest;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::yield_now;
use tokio::time::{Instant, sleep};
use tokio_util::task::TaskTracker;

use super::DiscoverySlots;
use super::clock::Clock;
use super::discover::{discover_server, discovery_needs, is_retryable};
use super::error::DiscoveryUnavailable;
use super::settings::{DiscoverySettings, ON_STARTUP_VARIABLE};
use super::slot::FailureReason;
use crate::oauth_discovery::fetch::{FetchError, FetchedDocument, MetadataFetcher};
use crate::oauth_discovery::outcome::DiscoveryStatus;
use crate::oauth_discovery::settings::{
    DiscoverySettings as FetchSettings, METADATA_TIMEOUT_VARIABLE,
};
use crate::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use crate::server_record::{MCPServer, OAuth2Flow};

include!("tests/support.rs");
include!("tests/settings.rs");
include!("tests/state.rs");
include!("tests/retry.rs");
include!("tests/concurrency.rs");
