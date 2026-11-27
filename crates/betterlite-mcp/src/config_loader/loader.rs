use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::Resolve;
use indexmap::IndexMap;

use super::checks::{
    CollisionCheck, config_identifier_owners, reject_config_server_id_collision,
    validate_config_server_names,
};
use super::config::McpServerConfig;
use super::dump::redacted_registry_dump;
use super::error::ConfigError;
use super::id::{generate_stable_server_id, pinned_config_server_id};
use super::plan::{self, OAuthPlan};
use super::ports::{DcrHydrator, InstructionsCache, OpenApiToolSink, ServerDefinitionCaches};
use super::record::{RecordInputs, ResolvedOAuth, build_mcp_info, build_record, config_scopes};
use crate::discovery_slots::DiscoverySlots;
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::registry::servers::SharedRegistry;
use crate::server_build::warnings::{
    warn_config_id_jag_server_outruns_sso, warn_legacy_delegate_auth_if_applicable,
    warn_on_server_name_fields,
};
use crate::server_build::{Decryptor, DeferredSlots, ServerBuilder, StdioPolicy};
use crate::server_oauth::unresolved::requires_discovery;
use crate::server_oauth::warnings::{UnresolvedCheck, warn_unresolved};
use crate::server_payload::rules::MCP_STDIO_DISABLED_MESSAGE;
use crate::server_record::{MCPOAuthMetadata, MCPServer};

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
            db_backed_server_ids: self.db_backed_server_ids().await,
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
        self.log_redacted_dump().await;
        self.hydrate_dcr_clients().await;
        self.prime_discovery().await;
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
            plan::validate_config(entry.config, &prepared.server_ref, &plan)?;
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
        let server = self.install_server(server, &plan).await?;
        self.register_configured_spec(entry, &server).await
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
            .or_else(|| gated.as_ref().and_then(|found| found.scopes.clone()));
        plan::resolved_oauth(plan, scopes, configured_scopes, gated)
    }

    async fn install_server(
        &self,
        mut server: MCPServer,
        plan: &OAuthPlan,
    ) -> Result<MCPServer, ConfigError> {
        let used_prefixes = self.used_prefixes(&server.fields.server_id).await;
        self.builder
            .assign_unique_short_prefix(&mut server, &used_prefixes)?;
        warn_legacy_delegate_auth_if_applicable(&server, "config", &self.builder.prefixes);
        warn_config_id_jag_server_outruns_sso(&server, self.id_jag_gap, &self.builder.prefixes);
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
        self.register_openapi_tools(spec_path, server, base_url)
            .await
    }

    pub async fn hydrate_dcr_clients(&self) {
        let server_ids: Vec<String> = self
            .registry
            .read()
            .await
            .config_servers
            .keys()
            .cloned()
            .collect();
        for server_id in server_ids {
            let stored = {
                let registry = self.registry.read().await;
                registry.config_servers.get(&server_id).cloned()
            };
            let Some(mut server) = stored else {
                continue;
            };
            if !self.dcr.hydrate(&mut server).await {
                continue;
            }
            if let Some(entry) = self
                .registry
                .write()
                .await
                .config_servers
                .get_mut(&server_id)
            {
                entry.clone_from(&server);
            }
            tracing::debug!(
                "hydrated persisted DCR client onto config MCP server server_id={server_id}"
            );
        }
    }

    async fn db_backed_server_ids(&self) -> BTreeSet<String> {
        self.registry
            .read()
            .await
            .db_servers
            .keys()
            .cloned()
            .collect()
    }

    async fn used_prefixes(&self, server_id: &str) -> BTreeMap<String, String> {
        let registry = self.registry.read().await;
        let mut used = BTreeMap::new();
        let servers = registry
            .config_servers
            .values()
            .chain(registry.db_servers.values());
        for server in servers {
            if server.fields.server_id == server_id {
                continue;
            }
            if let Some(prefix) = server
                .fields
                .short_prefix
                .as_ref()
                .filter(|prefix| !prefix.is_empty())
            {
                used.insert(prefix.clone(), server.fields.server_id.clone());
            }
        }
        used
    }

    async fn log_redacted_dump(&self) {
        let dump = {
            let registry = self.registry.read().await;
            redacted_registry_dump(&registry.config_servers)
        };
        let text = serde_json::to_string_pretty(&dump).unwrap_or_default();
        tracing::debug!("Loaded MCP Servers: {text}");
    }

    async fn prime_discovery(&self) {
        let servers: Vec<MCPServer> = self
            .registry
            .read()
            .await
            .config_servers
            .values()
            .cloned()
            .collect();
        self.slots.prime(&servers);
    }
}

// get_server_prefix on a temporary server: the alias when non-empty, else the entry's name
fn display_name(server_name: &str, alias: Option<&str>) -> String {
    match alias {
        Some(alias) if !alias.is_empty() => alias.to_owned(),
        Some(_) | None => server_name.to_owned(),
    }
}

fn resolve_server_id(
    entry: Entry<'_>,
    alias: Option<&str>,
    server_url: &str,
    state: &mut LoadState,
) -> Result<String, ConfigError> {
    let config = entry.config;
    let pinned = pinned_config_server_id(config.server_id.as_ref(), entry.server_name)?;
    let server_id = match &pinned {
        Some(pinned_id) => pinned_id.clone(),
        None => generate_stable_server_id(
            entry.server_name,
            server_url,
            config.transport,
            config.auth_type,
            alias,
        ),
    };
    reject_config_server_id_collision(CollisionCheck {
        assigned_server_ids: &state.assigned_server_ids,
        server_id: &server_id,
        server_name: entry.server_name,
        pinned: pinned.is_some(),
        db_backed_server_ids: &state.db_backed_server_ids,
        identifier_owners: &state.identifier_owners,
    })?;
    state
        .assigned_server_ids
        .insert(server_id.clone(), entry.server_name.to_owned());
    warn_on_server_name_fields(&server_id, alias, Some(entry.server_name));
    Ok(server_id)
}

fn select_alias(
    server_name: &str,
    config: &McpServerConfig,
    mcp_aliases: Option<&IndexMap<String, String>>,
    used_aliases: &mut BTreeSet<String>,
) -> Option<String> {
    if let Some(alias) = config.alias.as_ref() {
        return Some(alias.clone());
    }
    let aliases = mcp_aliases?;
    for (alias_name, target_server_name) in aliases {
        if target_server_name == server_name && !used_aliases.contains(alias_name) {
            used_aliases.insert(alias_name.clone());
            tracing::debug!("Mapped alias '{alias_name}' to server '{server_name}'");
            return Some(alias_name.clone());
        }
    }
    None
}

fn warn_stdio_blocked(server_name: &str, transport: Option<MCPTransport>, blocked: bool) {
    if transport != Some(MCPTransport::Stdio) || !blocked {
        return;
    }
    tracing::warn!("MCP server '{server_name}' will not start: {MCP_STDIO_DISABLED_MESSAGE}");
}

fn warn_oauth_unresolved(
    prepared: &Prepared,
    plan: &OAuthPlan,
    metadata: Option<&MCPOAuthMetadata>,
) {
    warn_unresolved(UnresolvedCheck {
        server_ref: &prepared.server_ref,
        server_url: Some(&prepared.server_url),
        discovery_attempted: plan.should_discover,
        discovery_deferred: plan.deferred,
        issuer_anchored: plan.use_issuer_anchor,
        metadata,
        needs_authorization_url: plan.needs_authorization_url,
        needs_token_url: plan.needs_token_url,
        manual_authorization_url: plan.endpoints.manual_authorization_url.as_deref(),
        manual_token_url: plan.endpoints.manual_token_url.as_deref(),
    });
}
