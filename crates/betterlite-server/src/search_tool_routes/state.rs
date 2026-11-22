use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::search::SearchRequest;
use betterlite_db::search_tools::convert::tool::SearchTool;
use betterlite_db::search_tools::registry::SearchToolRegistry;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_openai::search::records::SearchResponse;
use thiserror::Error;

pub struct SearchToolState<Table, Reconnect, Refresh> {
    // None answers LiteLLM's "Prisma client not initialized" on the five table routes
    pub database: Option<SearchToolDatabase<Table, Reconnect>>,
    pub refresh: Arc<Refresh>,
    // shared with test_connection and ui/available_providers, which take the ports alone as their state
    pub ports: Arc<SearchToolPorts>,
}

pub struct SearchToolDatabase<Table, Reconnect> {
    pub table: Table,
    // cloned per list read, since get_all_search_tools_from_db borrows it mutably
    pub reconnect: Reconnect,
    pub registry: SearchToolRegistry,
}

pub struct SearchToolPorts {
    pub config: Arc<dyn ConfigSearchTools>,
    pub teams: Arc<dyn TeamObjectLookup>,
    pub search: Arc<dyn ConnectionSearch>,
    pub catalog: Arc<dyn ProviderCatalog>,
}

pub type PortFuture<'call, Answer, Failure> =
    Pin<Box<dyn Future<Output = Result<Answer, Failure>> + Send + 'call>>;

// the config reader and the provider registry live in the binary crate, whose error types this crate cannot name
#[derive(Debug, Error)]
#[error("{source}")]
pub struct PortError {
    #[source]
    pub source: Box<dyn StdError + Send + Sync>,
}

// asearch's error type is fixed by the mount's provider lookup, which this crate cannot name
#[derive(Debug, Error)]
#[error("{source}")]
pub struct ConnectionFailure {
    // type(e).__name__ of the LiteLLM exception the search error converted to, such as AuthenticationError
    pub error_type: &'static str,
    #[source]
    pub source: Box<dyn StdError + Send + Sync>,
}

// proxy_config.parse_search_tools(await proxy_config.get_config()): the loaded file's search_tools list
pub trait ConfigSearchTools: Send + Sync {
    fn search_tools(&self) -> PortFuture<'_, Vec<SearchTool>, PortError>;
}

// B737's get_team_object for the caller's team, with the mount's cache and database
pub trait TeamObjectLookup: Send + Sync {
    fn team_object<'call>(
        &'call self,
        team_id: &'call str,
        caller: &'call UserAPIKeyAuth,
    ) -> PortFuture<'call, LiteLLMTeamTableCachedObj, TeamLookupError>;
}

// B3272's asearch with the mount's provider lookup, pool and logging context
pub trait ConnectionSearch: Send + Sync {
    fn search(&self, request: SearchRequest) -> PortFuture<'_, SearchResponse, ConnectionFailure>;
}

// ProviderConfigManager.get_provider_search_config(provider).ui_friendly_name(), None for an unmapped provider
pub trait ProviderCatalog: Send + Sync {
    fn ui_friendly_name(&self, provider: &str) -> Result<Option<String>, PortError>;
}
