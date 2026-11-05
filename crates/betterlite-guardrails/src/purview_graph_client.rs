use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use lru::LruCache;
use secrecy::SecretString;
use uuid::Uuid;

use crate::purview_graph_client::scope_cache::CachedScope;
use crate::purview_graph_client::token_cache::CachedToken;

pub mod completion_prompt;
pub mod error;
pub mod graph_api;
pub mod message_concat;
pub mod policy_action;
pub mod process_content;
pub mod scope_cache;
pub mod token_cache;
pub mod tool_call_extraction;
pub mod user_id_resolution;
pub mod wire;

mod test_support;

pub const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
pub const LOGIN_AUTHORITY: &str = "https://login.microsoftonline.com";
pub const GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default";
pub const DEFAULT_APP_NAME: &str = "LiteLLM";
pub const DEFAULT_USER_ID_FIELD: &str = "user_id";

// One hour, the cache lifetime Microsoft recommends for protection scopes.
pub const SCOPE_CACHE_TTL: Duration = Duration::from_secs(3600);
pub const TOKEN_EXPIRY_MARGIN: Duration = Duration::from_secs(60);
const SCOPE_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(1000) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};

#[derive(Debug)]
pub struct CacheState {
    oauth_token_cache: Option<CachedToken>,
    protection_scope_cache: LruCache<String, CachedScope>,
}

impl Default for CacheState {
    fn default() -> Self {
        Self {
            oauth_token_cache: None,
            protection_scope_cache: LruCache::new(SCOPE_CACHE_CAPACITY),
        }
    }
}

// std Mutex, not tokio's: the proxy loop and the logging thread's own runtime both lock it, and nothing awaits under it.
#[derive(Debug)]
pub struct PurviewGuardrailBase {
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub purview_app_name: String,
    pub user_id_field: String,
    pub timeout: Duration,
    pub client: reqwest::Client,
    pub graph_api_base: String,
    pub login_authority: String,
    pub clock: fn() -> SystemTime,
    pub new_id: fn() -> Uuid,
    pub scope_and_token_cache_lock: Mutex<CacheState>,
}
