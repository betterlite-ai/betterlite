use betterlite_http::user_url::validate::Resolve;
use betterlite_prompts::arguments::json::Object;

use super::RegistryMutator;
use super::error::MutationError;
use super::ports::{CacheInvalidator, ServerRows, ToolLister, ToolRegistrySink};
use crate::config_loader::loader::ConfigLoader;
use crate::config_loader::ports::{
    DcrHydrator, InstructionsCache, OpenApiToolSink, ServerDefinitionCaches,
};
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::openapi_tools::tool::OpenApiToolConfig;
use crate::server_build::{Decryptor, DeferredSlots, ServerBuilder, StdioPolicy};
use crate::server_record::MCPServer;

// LiteLLM registers with initialize_mapping=False, so the caller runs the mapping rebuild itself
struct RegistrationSink<'sink, T> {
    inner: &'sink T,
}

impl<T> OpenApiToolSink for RegistrationSink<'_, T>
where
    T: OpenApiToolSink,
{
    fn register_tool(
        &self,
        name: &str,
        description: &str,
        input_schema: Object,
        config: OpenApiToolConfig,
    ) {
        self.inner
            .register_tool(name, description, input_schema, config);
    }

    fn set_tool_owner(&self, tool_name: &str, server_prefix: &str) {
        self.inner.set_tool_owner(tool_name, server_prefix);
    }

    fn rebuild_tool_name_mapping(&self) {}
}

struct CacheSink<'cache, X> {
    inner: &'cache X,
}

impl<X> ServerDefinitionCaches for CacheSink<'_, X>
where
    X: CacheInvalidator,
{
    // only the config load path invalidates through the loader; the reload invalidates in publish
    #[cfg_attr(test, mutants::skip)]
    fn invalidate(&self, server_id: &str) {
        self.inner.invalidate_server(server_id);
    }
}

impl<D, F, S, R, C, Rows, H, L, T, I, X, U>
    RegistryMutator<'_, D, F, S, R, C, Rows, H, L, T, I, X, U>
where
    D: Decryptor,
    F: MetadataFetcher + 'static,
    S: StdioPolicy,
    R: DeferredSlots,
    C: Clock + Send + Sync + 'static,
    Rows: ServerRows,
    H: DcrHydrator,
    L: ToolLister + 'static,
    T: ToolRegistrySink,
    I: InstructionsCache,
    X: CacheInvalidator,
    U: Resolve,
{
    // LiteLLM's _maybe_register_openapi_tools: true when a spec was registered
    pub(super) async fn register_openapi_tools(
        &self,
        server: &MCPServer,
    ) -> Result<bool, MutationError> {
        if server
            .fields
            .spec_path
            .as_deref()
            .is_none_or(|path| path.is_empty())
        {
            return Ok(false);
        }
        let sink = RegistrationSink { inner: self.tools };
        let cache_sink = CacheSink { inner: self.caches };
        // reborrow the builder at this call's lifetime, the one ConfigLoader ties its fields to
        let builder = ServerBuilder {
            decryptor: self.builder.decryptor,
            env_cipher: self.builder.env_cipher,
            fetcher: self.builder.fetcher,
            settings: self.builder.settings,
            stdio: self.builder.stdio,
            slots: self.builder.slots,
            prefixes: self.builder.prefixes.clone(),
        };
        let loader = ConfigLoader {
            builder: &builder,
            slots: &self.slots,
            registry: self.registry.clone(),
            dcr: self.dcr,
            tools: &sink,
            instructions: self.instructions,
            caches: &cache_sink,
            spec_fetcher: self.spec_fetcher,
            client: self.client,
            id_jag_gap: None,
        };
        loader.maybe_register_openapi_tools(server).await?;
        Ok(true)
    }
}
