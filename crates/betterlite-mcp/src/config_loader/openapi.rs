use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::MCPAuth;
use secrecy::ExposeSecret as _;

use super::error::ConfigError;
use super::loader::ConfigLoader;
use super::ports::{DcrHydrator, InstructionsCache, OpenApiToolSink, ServerDefinitionCaches};
use crate::credentials::{McpAuthType, strip_auth_scheme, to_basic_credentials};
use crate::discovery_slots::clock::Clock;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::openapi_tools::base_url::get_base_url;
use crate::openapi_tools::document::{Components, OpenApiDocument, Operation, PathItem};
use crate::openapi_tools::error::OpenApiError;
use crate::openapi_tools::input_schema::build_input_schema;
use crate::openapi_tools::parameters::resolve_operation_params;
use crate::openapi_tools::spec_loading::load_openapi_spec_async;
use crate::openapi_tools::tool::OpenApiToolConfig;
use crate::server_build::{Decryptor, DeferredSlots, StdioPolicy};
use crate::server_payload::quoted_list;
use crate::server_record::MCPServer;
use crate::server_utils::headers::merge_mcp_headers;
use crate::server_utils::names::{add_server_prefix_to_name, openapi_tool_name};
use crate::server_utils::prefixes::get_server_prefix;
use betterlite_http::user_url::validate::Resolve;

const OPENAPI_METHODS: [&str; 5] = ["get", "post", "put", "delete", "patch"];
const AUTHORIZATION_HEADER: &str = "Authorization";

struct SpecContext<'spec> {
    spec: &'spec OpenApiDocument,
    base_url: &'spec str,
    headers: &'spec BTreeMap<String, String>,
    server: &'spec MCPServer,
    server_prefix: &'spec str,
}

struct OperationContext<'operation> {
    path: &'operation str,
    method: &'operation str,
    path_item: &'operation PathItem,
    operation: &'operation Operation,
    components: &'operation Components,
    base_url: &'operation str,
    headers: &'operation BTreeMap<String, String>,
    server: &'operation MCPServer,
    server_prefix: &'operation str,
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
    pub async fn register_openapi_tools(
        &self,
        spec_path: &str,
        server: &MCPServer,
        base_url: &str,
    ) -> Result<(), ConfigError> {
        if let Err(source) = self.register_spec_tools(spec_path, server, base_url).await {
            tracing::error!(
                "Failed to register OpenAPI tools for server {}: {source}",
                server.fields.name
            );
            return Err(ConfigError::OpenApi {
                server: server.fields.name.clone(),
                source,
            });
        }
        Ok(())
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
        self.register_openapi_tools(spec_path, server, base_url)
            .await?;
        self.tools.rebuild_tool_name_mapping();
        Ok(())
    }

    async fn register_spec_tools(
        &self,
        spec_path: &str,
        server: &MCPServer,
        base_url: &str,
    ) -> Result<(), OpenApiError> {
        let spec = load_openapi_spec_async(self.spec_fetcher, self.client, spec_path, None).await?;
        let base_url = if base_url.is_empty() {
            get_base_url(&spec, Some(spec_path))
        } else {
            base_url.to_owned()
        };
        tracing::info!(
            "Registering OpenAPI tools for server {} with base URL: {base_url}",
            server.fields.name
        );
        let server_prefix = get_server_prefix(server, &self.builder.prefixes);
        let headers = openapi_headers(server);
        tracing::debug!(
            "Using headers for OpenAPI tools (excluding sensitive values): {}",
            quoted_list(headers.keys())
        );
        let context = SpecContext {
            spec: &spec,
            base_url: &base_url,
            headers: &headers,
            server,
            server_prefix: &server_prefix,
        };
        let mut registered = 0_usize;
        for (path, path_item) in &spec.paths {
            registered += self.register_path(&context, path, path_item);
        }
        tracing::info!(
            "Successfully registered {registered} OpenAPI tools for server {}",
            server.fields.name
        );
        Ok(())
    }

    fn register_path(&self, context: &SpecContext<'_>, path: &str, path_item: &PathItem) -> usize {
        let mut registered = 0_usize;
        for method in OPENAPI_METHODS {
            let Some(operation) = path_item.operations.get(method) else {
                continue;
            };
            self.register_operation(&OperationContext {
                path,
                method,
                path_item,
                operation,
                components: &context.spec.components,
                base_url: context.base_url,
                headers: context.headers,
                server: context.server,
                server_prefix: context.server_prefix,
            });
            registered += 1;
        }
        registered
    }

    fn register_operation(&self, context: &OperationContext<'_>) {
        let resolved =
            resolve_operation_params(context.operation, context.path_item, context.components);
        let raw_operation_id = operation_id(context);
        let base_tool_name = openapi_tool_name(&raw_operation_id);
        let prefixed_tool_name = add_server_prefix_to_name(
            &base_tool_name,
            context.server_prefix,
            &self.builder.prefixes.separator,
        );
        let description = operation_description(context);
        let input_schema = build_input_schema(&resolved);
        let config = tool_config(context, resolved);
        self.tools
            .register_tool(&prefixed_tool_name, &description, input_schema, config);
        self.tools
            .set_tool_owner(&base_tool_name, context.server_prefix);
        self.tools
            .set_tool_owner(&prefixed_tool_name, context.server_prefix);
        tracing::debug!(
            "Registered OpenAPI tool: {prefixed_tool_name} for server {}",
            context.server.fields.name
        );
    }
}

fn operation_id(context: &OperationContext<'_>) -> String {
    match context.operation.operation_id.as_deref() {
        Some(operation_id) => operation_id.to_owned(),
        None => format!("{}_{}", context.method, context.path.replace('/', "_")),
    }
}

fn operation_description(context: &OperationContext<'_>) -> String {
    match context
        .operation
        .summary
        .clone()
        .or_else(|| context.operation.description.clone())
    {
        Some(description) => description,
        None => format!("{} {}", context.method.to_uppercase(), context.path),
    }
}

fn tool_config(context: &OperationContext<'_>, resolved: Operation) -> OpenApiToolConfig {
    OpenApiToolConfig {
        path: context.path.to_owned(),
        method: context.method.to_owned(),
        operation: resolved,
        base_url: context.base_url.to_owned(),
        headers: context.headers.clone(),
        server_label: Some(server_label(context.server)),
        relays_upstream_auth: context.server.is_client_forwarded_token(),
        auth_type: openapi_auth_type(context.server.fields.auth_type),
        upstream_token_header: context.server.fields.upstream_token_header.clone(),
    }
}

fn openapi_headers(server: &MCPServer) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    let token = server
        .fields
        .authentication_token
        .as_ref()
        .map(|credential| credential.0.0.expose_secret())
        .filter(|token| !token.is_empty());
    if let Some(token) = token {
        let value = match server.fields.auth_type {
            Some(MCPAuth::BearerToken) => {
                Some(format!("Bearer {}", strip_auth_scheme(token, "Bearer")))
            }
            Some(MCPAuth::ApiKey) => Some(format!("ApiKey {}", strip_auth_scheme(token, "ApiKey"))),
            Some(MCPAuth::Basic) => Some(format!("Basic {}", to_basic_credentials(token))),
            Some(MCPAuth::Token) => Some(format!("token {}", strip_auth_scheme(token, "token"))),
            Some(
                MCPAuth::None
                | MCPAuth::Authorization
                | MCPAuth::Oauth2
                | MCPAuth::AwsSigv4
                | MCPAuth::Oauth2TokenExchange
                | MCPAuth::Oauth2IdJag
                | MCPAuth::TruePassthrough
                | MCPAuth::OauthDelegate,
            )
            | None => None,
        };
        if let Some(value) = value {
            headers.insert(String::from(AUTHORIZATION_HEADER), value);
        }
    }
    merge_mcp_headers(Some(&headers), server.fields.static_headers.as_ref()).unwrap_or_default()
}

fn server_label(server: &MCPServer) -> String {
    let candidates = [
        Some(server.fields.name.as_str()),
        server.fields.server_name.as_deref(),
        server.fields.alias.as_deref(),
        Some(server.fields.server_id.as_str()),
    ];
    for candidate in candidates.into_iter().flatten() {
        if !candidate.is_empty() {
            return candidate.to_owned();
        }
    }
    server.fields.server_id.clone()
}

#[must_use]
const fn openapi_auth_type(auth_type: Option<MCPAuth>) -> Option<McpAuthType> {
    match auth_type {
        Some(MCPAuth::None) => Some(McpAuthType::None),
        Some(MCPAuth::ApiKey) => Some(McpAuthType::ApiKey),
        Some(MCPAuth::BearerToken) => Some(McpAuthType::BearerToken),
        Some(MCPAuth::Basic) => Some(McpAuthType::Basic),
        Some(MCPAuth::Authorization) => Some(McpAuthType::Authorization),
        Some(MCPAuth::Oauth2) => Some(McpAuthType::Oauth2),
        Some(MCPAuth::Token) => Some(McpAuthType::Token),
        Some(MCPAuth::Oauth2TokenExchange) => Some(McpAuthType::Oauth2TokenExchange),
        Some(
            MCPAuth::AwsSigv4
            | MCPAuth::Oauth2IdJag
            | MCPAuth::TruePassthrough
            | MCPAuth::OauthDelegate,
        )
        | None => None,
    }
}
