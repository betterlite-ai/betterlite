use std::collections::BTreeSet;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use futures_util::future::join_all;
use tracing::warn;

use super::ports::{AllowedServers, Clock, EnvVarCheck, ProbeClient, SessionProbe, SpecLoader};
use super::service::HealthService;
use crate::registry::lookup::ServerLookups as _;
use crate::registry::table::ServerTables as _;

pub const BATCH_SIZE: usize = 10;

impl<P, S, E, A, C, L> HealthService<P, S, E, A, C, L>
where
    P: ProbeClient,
    S: SessionProbe,
    E: EnvVarCheck,
    A: AllowedServers,
    C: Clock,
    L: SpecLoader,
{
    #[must_use]
    pub async fn with_health_and_teams(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        server_ids: Option<&[String]>,
        checked_server_ids: &BTreeSet<String>,
    ) -> Vec<McpServerTable> {
        let allowed = self.deps.allowed.allowed_server_ids(auth);
        let wanted: Vec<String> = match server_ids.filter(|ids| !ids.is_empty()) {
            Some(ids) => ids
                .iter()
                .filter(|server_id| allowed.contains(*server_id))
                .cloned()
                .collect(),
            None => allowed,
        };
        let unchecked: Vec<String> = wanted
            .into_iter()
            .filter(|server_id| !checked_server_ids.contains(server_id))
            .collect();
        self.run_health_checks(&unchecked).await
    }

    #[must_use]
    pub async fn with_health_unfiltered(
        &self,
        server_ids: Option<&[String]>,
    ) -> Vec<McpServerTable> {
        let registered: Vec<String> = {
            let registry = self.deps.registry.read().await;
            registry
                .get_registry()
                .into_iter()
                .map(|server| server.fields.server_id.clone())
                .collect()
        };
        if registered.is_empty() {
            return Vec::new();
        }
        let target: Vec<String> = match server_ids.filter(|ids| !ids.is_empty()) {
            Some(ids) => ids
                .iter()
                .filter(|server_id| registered.contains(*server_id))
                .cloned()
                .collect(),
            None => registered,
        };
        if target.is_empty() {
            return Vec::new();
        }
        self.run_health_checks(&target).await
    }

    #[must_use]
    pub async fn run_health_checks(&self, target_server_ids: &[String]) -> Vec<McpServerTable> {
        let mut unique_ids: Vec<&String> = Vec::new();
        for server_id in target_server_ids {
            if !unique_ids.contains(&server_id) {
                unique_ids.push(server_id);
            }
        }
        let mut rows = Vec::with_capacity(unique_ids.len());
        for batch in unique_ids.chunks(BATCH_SIZE) {
            let checks = batch
                .iter()
                .map(|server_id| self.check_server(server_id.as_str()));
            rows.extend(join_all(checks).await);
        }
        rows
    }

    #[must_use]
    pub async fn all_allowed(&self, auth: Option<&UserAPIKeyAuth>) -> Vec<McpServerTable> {
        let allowed = self.deps.allowed.allowed_server_ids(auth);
        let registry = self.deps.registry.read().await;
        let mut rows = Vec::with_capacity(allowed.len());
        for server_id in &allowed {
            let Some(server) = registry.server_by_id(server_id, None) else {
                warn!("MCP Server {server_id} not found in registry");
                continue;
            };
            rows.push(registry.server_table(server));
        }
        rows
    }
}
