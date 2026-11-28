mod prepare;
mod state;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use indexmap::IndexMap;

use self::prepare::{
    display_name, resolve_server_id, select_alias, warn_oauth_unresolved, warn_stdio_blocked,
};
use self::state::{hydrate_dcr_clients, log_redacted_dump, prime_discovery, used_prefixes};
use super::checks::{config_identifier_owners, validate_config, validate_config_server_names};
use super::config::McpServerConfig;
use super::error::ConfigError;
use super::openapi::OpenApiRegistrar;
use super::plan::{self, OAuthPlan};
use super::ports::{DcrHydrator, InstructionsCache, OpenApiToolSink, ServerDefinitionCaches};
use super::record::{RecordInputs, ResolvedOAuth, build_mcp_info, build_record, config_scopes};
use crate::discovery_slots::DiscoverySlots;
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::registry::servers::SharedRegistry;
use crate::server_build::warnings::{
    // ast-grep-ignore: no-long-names
    warn_config_id_jag_server_outruns_sso as warn_id_jag_gap,
    // ast-grep-ignore: no-long-names
    warn_legacy_delegate_auth_if_applicable as warn_delegate_auth,
};
use crate::server_build::{Decryptor, DeferredSlots, ServerBuilder, StdioPolicy};
use crate::server_oauth::unresolved::requires_discovery;
use crate::server_record::MCPServer;

pub struct ConfigLoader<'loader, D, F, S, R, C, H, T, I, X, U> {
    pub builder: &'loader ServerBuilder<'loader, D, F, S, R>,
    pub slots: &'loader Arc<DiscoverySlots<F, C>>,
    pub registry: SharedRegistry,
    pub dcr: &'loader H,
    pub tools: &'loader T,
    pub instructions: &'loader I,
    pub caches: &'loader X,
    pub spec_fetcher: &'loader UserUrlFetcher<U>,
    pub client: &'loader ProxiedClient,
    pub id_jag_gap: Option<&'loader str>,
}

#[derive(Clone, Copy)]
struct Entry<'entry> {
    server_name: &'entry str,
    config: &'entry McpServerConfig,
    aliases: Option<&'entry IndexMap<String, String>>,
}

struct Prepared {
    mcp_info: MCPInfo,
    alias: Option<String>,
    name: String,
    server_id: String,
    server_url: String,
    server_ref: String,
}

struct LoadState {
    used_aliases: BTreeSet<String>,
    assigned_server_ids: BTreeMap<String, String>,
    db_backed_server_ids: BTreeSet<String>,
    identifier_owners: BTreeMap<String, BTreeSet<String>>,
}

impl<D, F, S, R, C, H, T, I, X, U> ConfigLoader<'_, D, F, S, R, C, H, T, I, X, U>
where
    D: Decryptor,
    F: MetadataFetcher + 'static,
    S: StdioPolicy,
    R: DeferredSlots,
    C: Clock + Send + Sync + 'static,
    H: DcrHydrator,
    T: OpenApiToolSink,
    I: InstructionsCache,
    X: ServerDefinitionCaches,
    U: Resolve,
{
    pub async fn load(
        &self,
        config: &IndexMap<String, McpServerConfig>,
        mcp_aliases: Option<&IndexMap<String, String>>,
    ) -> Result<(), ConfigError> {
        tracing::debug!("Loading MCP Servers from config-----");
        self.instructions.clear();
        validate_config_server_names(config, &self.builder.prefixes.separator)?;
        let mut state = LoadState {
            used_aliases: BTreeSet::new(),
            assigned_server_ids: BTreeMap::new(),
            db_backed_server_ids: self
                .registry
                .read()
                .await
                .db_servers
                .keys()
                .cloned()
                .collect(),
            identifier_owners: config_identifier_owners(config, mcp_aliases),
        };
        for (server_name, server_config) in config {
            let entry = Entry {
                server_name: server_name.as_str(),
                config: server_config,
                aliases: mcp_aliases,
            };
            self.load_entry(entry, &mut state).await?;
        }
        log_redacted_dump(&self.registry).await;
        hydrate_dcr_clients(&self.registry, self.dcr).await;
        prime_discovery(&self.registry, self.slots).await;
        self.tools.rebuild_tool_name_mapping();
        Ok(())
    }

    async fn load_entry(&self, entry: Entry<'_>, state: &mut LoadState) -> Result<(), ConfigError> {
        let prepared = self.prepare_entry(entry, state)?;
        let plan = plan::oauth_plan(
            entry.config,
            &prepared.server_ref,
            &prepared.server_url,
            self.builder.settings.on_startup,
        );
        let oauth = self.resolve_oauth(entry.config, &prepared, &plan).await;
        let (per_server_oauth_discovery, dcr_bridge) =
            validate_config(entry.config, &prepared.server_ref, &plan)?;
        warn_oauth_unresolved(&prepared, &plan, oauth.metadata.as_ref());
        let server = build_record(RecordInputs {
            config: entry.config,
            server_name: entry.server_name,
            server_url: &prepared.server_url,
            server_id: &prepared.server_id,
            name: &prepared.name,
            alias: prepared.alias.as_deref(),
            mcp_info: prepared.mcp_info,
            oauth,
            per_server_oauth_discovery,
            dcr_bridge,
        })?;
        let installed = self.install_server(server, &plan).await?;
        self.register_configured_spec(entry, &installed).await
    }

    fn prepare_entry(
        &self,
        entry: Entry<'_>,
        state: &mut LoadState,
    ) -> Result<Prepared, ConfigError> {
        let config = entry.config;
        let mcp_info = build_mcp_info(entry.server_name, config)?;
        let alias = select_alias(
            entry.server_name,
            config,
            entry.aliases,
            &mut state.used_aliases,
        );
        let name = display_name(entry.server_name, alias.as_deref());
        let server_url = config.url.clone().unwrap_or_default();
        let server_id = resolve_server_id(entry, alias.as_deref(), &server_url, state)?;
        warn_stdio_blocked(
            entry.server_name,
            config.transport,
            self.builder.stdio.is_blocked(),
        );
        let server_ref = if entry.server_name.is_empty() {
            server_id.clone()
        } else {
            entry.server_name.to_owned()
        };
        Ok(Prepared {
            mcp_info,
            alias,
            name,
            server_id,
            server_url,
            server_ref,
        })
    }

    async fn resolve_oauth(
        &self,
        config: &McpServerConfig,
        prepared: &Prepared,
        plan: &OAuthPlan,
    ) -> ResolvedOAuth {
        let metadata = plan::fetch_metadata(
            self.builder.fetcher,
            plan,
            self.builder.settings.fetch,
            &prepared.server_url,
        )
        .await;
        let gated = plan::gate_metadata(
            plan,
            &prepared.server_ref,
            config.dcr_bridge.as_ref(),
            metadata,
        );
        let configured_scopes = config_scopes(config);
        let scopes = configured_scopes
            .clone()
            .or_else(|| gated.as_ref()?.scopes.clone());
        plan::resolved_oauth(plan, scopes, configured_scopes, gated)
    }

    async fn install_server(
        &self,
        mut server: MCPServer,
        plan: &OAuthPlan,
    ) -> Result<MCPServer, ConfigError> {
        let used = used_prefixes(&self.registry, &server.fields.server_id).await;
        self.builder
            .assign_unique_short_prefix(&mut server, &used)?;
        warn_delegate_auth(&server, "config", &self.builder.prefixes);
        warn_id_jag_gap(&server, self.id_jag_gap, &self.builder.prefixes);
        self.caches.invalidate(&server.fields.server_id);
        self.registry
            .write()
            .await
            .config_servers
            .insert(server.fields.server_id.clone(), server.clone());
        self.slots.set_deferred(
            &server.fields.server_id,
            requires_discovery(
                Some(server.fields.url.as_deref().unwrap_or("")),
                plan.use_issuer_anchor,
                &server,
            ),
        );
        Ok(server)
    }

    async fn register_configured_spec(
        &self,
        entry: Entry<'_>,
        server: &MCPServer,
    ) -> Result<(), ConfigError> {
        let Some(spec_path) = entry
            .config
            .spec_path
            .as_deref()
            .filter(|path| !path.is_empty())
        else {
            return Ok(());
        };
        tracing::info!(
            "Loading OpenAPI spec from {spec_path} for server {}",
            entry.server_name
        );
        let base_url = entry.config.url.as_deref().unwrap_or("");
        self.openapi_registration()
            .register_openapi_tools(spec_path, server, base_url)
            .await
    }

    pub async fn maybe_register_openapi_tools(
        &self,
        server: &MCPServer,
    ) -> Result<(), ConfigError> {
        let Some(spec_path) = server
            .fields
            .spec_path
            .as_deref()
            .filter(|path| !path.is_empty())
        else {
            return Ok(());
        };
        tracing::info!(
            "Loading OpenAPI spec from {spec_path} for server {}",
            server.fields.name
        );
        let base_url = server.fields.url.as_deref().unwrap_or("");
        self.openapi_registration()
            .register_openapi_tools(spec_path, server, base_url)
            .await?;
        self.tools.rebuild_tool_name_mapping();
        Ok(())
    }

    fn openapi_registration(&self) -> OpenApiRegistrar<'_, T, U> {
        OpenApiRegistrar {
            tools: self.tools,
            spec_fetcher: self.spec_fetcher,
            client: self.client,
            prefixes: &self.builder.prefixes,
        }
    }
}
