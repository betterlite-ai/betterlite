use betterlite_db::managed_resource_rows::mcp::McpServerTable;

use super::credentials::resolved_credentials;
use super::fields::{RecordParts, exposed_map, name_for_prefix, record_mcp_info, server_fields};
// ast-grep-ignore: no-long-names
use super::warnings::warn_legacy_delegate_auth_if_applicable;
use super::{BuildError, BuildOptions, Decryptor, DeferredSlots, ServerBuilder, StdioPolicy};
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_oauth::unresolved::requires_discovery;
use crate::server_record::MCPServer;

impl<D, F, S, R> ServerBuilder<'_, D, F, S, R>
where
    D: Decryptor,
    F: MetadataFetcher,
    S: StdioPolicy,
    R: DeferredSlots,
{
    pub async fn build_from_table(
        &self,
        row: &McpServerTable,
        options: BuildOptions,
    ) -> Result<MCPServer, BuildError> {
        let encrypted = options.credentials_are_encrypted;
        let env_encrypted = options.env_vars_are_encrypted.unwrap_or(encrypted);
        let credentials = resolved_credentials(self, row.credentials.as_ref(), encrypted);
        let env_vars = self.resolve_env_vars_list(row.env_vars.as_deref(), env_encrypted)?;
        let name = name_for_prefix(row);
        let mcp_info = record_mcp_info(row)?;
        let oauth = self.table_oauth(row, &credentials, &name).await;
        let env = exposed_map(&row.env.0);
        let static_headers = row
            .static_headers
            .as_ref()
            .map(|field| exposed_map(&field.0));
        let parts = RecordParts {
            name,
            mcp_info,
            env: (!env.is_empty()).then_some(env),
            static_headers: static_headers.filter(|headers| !headers.is_empty()),
            env_vars,
            credentials,
            oauth,
        };
        let server = MCPServer::try_from(server_fields(row, parts)?)?;
        // ast-grep-ignore: no-long-names
        warn_legacy_delegate_auth_if_applicable(&server, "database", &self.prefixes);
        let deferred = requires_discovery(
            row.url.as_deref(),
            server.fields.issuer_is_anchored,
            &server,
        );
        self.slots
            .record_deferred(&server.fields.server_id, deferred);
        Ok(server)
    }
}
