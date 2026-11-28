use std::collections::BTreeMap;

use betterlite_mcp::oauth_resource::base_url::RequestOrigin;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::admission_fallback::challenge::{
    ChallengeContext, ChallengeResponse, McpAuthHeaders, bridge_invalid_token_challenge,
    gateway_dcr_challenge, has_client_supplied_mcp_auth, is_dcr_challenge_scope,
};
use betterlite_server::mcp::admission_fallback::targets::TargetQuery;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    ClientAuth,
    GatewayChallenge,
    BridgeChallenge,
    ChallengeScope,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    function: Function,
    #[serde(default)]
    mcp_auth_header: Option<String>,
    #[serde(default)]
    server_headers: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    route: String,
    #[serde(default)]
    mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    invalid_token: bool,
    #[serde(default)]
    requested_name: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    original_path: Option<String>,
    #[serde(default)]
    app_root_path: Option<String>,
    #[serde(default)]
    root_path: Option<String>,
    #[serde(default)]
    server_root_path: String,
    #[serde(default)]
    proxy_base_url: Option<String>,
    #[serde(default)]
    x_forwarded_proto: Option<String>,
    #[serde(default)]
    x_forwarded_host: Option<String>,
    #[serde(default)]
    x_forwarded_port: Option<String>,
    #[serde(default)]
    from_trusted_proxy: bool,
    #[serde(default)]
    error_status: u16,
}

impl Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = empty_registry();
        let query = TargetQuery {
            registry: &registry,
            route: &input.route,
            header_servers: input.mcp_servers.as_deref(),
            client_ip: None,
        };
        let ctx = challenge_context(input);
        let auth = McpAuthHeaders {
            mcp_auth_header: input.mcp_auth_header.as_deref(),
            server_headers: &input.server_headers,
        };
        match input.function {
            Function::ClientAuth => Ok(json!({
                "value": has_client_supplied_mcp_auth(auth.mcp_auth_header, auth.server_headers),
            })),
            Function::GatewayChallenge => Ok(response_value(&gateway_dcr_challenge(
                &ctx,
                &query,
                input.invalid_token,
            ))),
            Function::BridgeChallenge => Ok(response_value(&bridge_invalid_token_challenge(
                &ctx,
                &input.requested_name,
            ))),
            Function::ChallengeScope => Ok(json!({
                "value": is_dcr_challenge_scope(&query, &auth, input.error_status == 401_u16),
            })),
        }
    }
}

fn empty_registry() -> ServerRegistry {
    ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: true,
            internal_ip_ranges: None,
        },
        Vec::new(),
        Vec::new(),
    )
}

fn challenge_context(input: &Call) -> ChallengeContext<'_> {
    ChallengeContext {
        proxy_base_url: input.proxy_base_url.as_deref(),
        origin: RequestOrigin {
            base_url: &input.base_url,
            x_forwarded_proto: input.x_forwarded_proto.as_deref(),
            x_forwarded_host: input.x_forwarded_host.as_deref(),
            x_forwarded_port: input.x_forwarded_port.as_deref(),
            from_trusted_proxy: input.from_trusted_proxy,
        },
        server_root_path: &input.server_root_path,
        original_path: input.original_path.as_deref(),
        path: &input.path,
        app_root_path: input.app_root_path.as_deref(),
        root_path: input.root_path.as_deref(),
    }
}

fn response_value(response: &ChallengeResponse) -> Value {
    let status = response.status;
    let detail = &response.detail;
    let headers = &response.headers;
    json!({"status": status, "detail": detail, "headers": headers})
}
