pub mod body;
pub(crate) mod error;
pub(crate) mod flows;
pub(crate) mod management;
pub mod ports;
pub mod router;
pub(crate) mod status;
pub mod types;

mod tests;

use std::sync::Arc;

use betterlite_config::sections::mcp_management_mode::ManagementMode;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_mcp::registry::resolution::{DbLookup, ServerAccessCheck};
use betterlite_mcp::registry::servers::SharedRegistry;

use crate::mcp::management_server_access::AccessibleServerSources;

// The ports the mount task fills: no route reads a global, and the store and the cache are only contracts here
pub struct UserEnvRoutesState<Access, Sources>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    pub database: Option<Arc<PostgresStore>>,
    pub store: Arc<dyn ports::UserEnvVarsStore>,
    pub cache: Arc<dyn ports::UserEnvVarsCache>,
    pub registry: SharedRegistry,
    pub db_lookup: Arc<dyn DbLookup>,
    pub access: Arc<Access>,
    pub sources: Arc<Sources>,
    pub mode: ManagementMode,
    pub proxy_base_url: Option<String>,
}

impl<Access, Sources> Clone for UserEnvRoutesState<Access, Sources>
where
    Access: ServerAccessCheck,
    Sources: AccessibleServerSources,
{
    fn clone(&self) -> Self {
        Self {
            database: self.database.clone(),
            store: Arc::clone(&self.store),
            cache: Arc::clone(&self.cache),
            registry: Arc::clone(&self.registry),
            db_lookup: Arc::clone(&self.db_lookup),
            access: Arc::clone(&self.access),
            sources: Arc::clone(&self.sources),
            mode: self.mode,
            proxy_base_url: self.proxy_base_url.clone(),
        }
    }
}
