use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp_stored_contracts::MCPCredentials;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_payload::request::NewMCPServerRequest;
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::admin_credential_view::CredentialViewError;
use betterlite_server::mcp::management_visibility::redact_mcp_credentials_list;
use betterlite_server::mcp::management_visibility::user_is_full_admin;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::management_visibility::resolve_credential_target_user_id as target_user_id;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::management_visibility::has_non_admin_config_credentials as has_config_credentials;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::management_visibility::inherit_credentials_from_existing_server as inherit_credentials;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::management_visibility::sanitize_mcp_server_for_non_admin as non_admin;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::management_visibility::sanitize_mcp_server_for_virtual_key as virtual_key;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::management_visibility::sanitize_mcp_server_list_for_non_admin as non_admin_list;
// ast-grep-ignore: no-long-names
use betterlite_server::mcp::management_visibility::sanitize_mcp_server_list_for_virtual_key as virtual_key_list;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryInput {
    #[serde(default)]
    config_servers: Vec<MCPServer>,
    #[serde(default)]
    db_servers: Vec<MCPServer>,
    #[serde(default)]
    public_mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    public_hub_strict: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RedactList {
        registry: RegistryInput,
        servers: Vec<McpServerTable>,
    },
    NonAdmin {
        registry: RegistryInput,
        server: McpServerTable,
    },
    NonAdminList {
        registry: RegistryInput,
        servers: Vec<McpServerTable>,
    },
    VirtualKey {
        registry: RegistryInput,
        server: McpServerTable,
    },
    VirtualKeyList {
        registry: RegistryInput,
        servers: Vec<McpServerTable>,
    },
    FullAdmin {
        auth: BTreeMap<String, JsonData>,
    },
    CredentialTarget {
        auth: BTreeMap<String, JsonData>,
        requested_user_id: Option<String>,
    },
    HasNonAdminCredentials {
        credentials: Option<MCPCredentials>,
    },
    InheritCredentials {
        registry: RegistryInput,
        payload: NewMCPServerRequest,
    },
}

struct Fixture {
    registry: ServerRegistry,
    public_ids: Vec<String>,
}

impl RegistryInput {
    fn fixture(&self) -> Fixture {
        let public_ids = self.public_mcp_servers.clone().unwrap_or_default();
        let registry = ServerRegistry::new(
            RegistrySettings {
                public_mcp_servers: public_ids.clone(),
                public_hub_strict: self.public_hub_strict,
                internal_ip_ranges: None,
            },
            self.config_servers.clone(),
            self.db_servers.clone(),
        );
        Fixture {
            registry,
            public_ids,
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RedactList { registry, servers } => redacted_list(registry, servers),
            Input::NonAdmin { registry, server } => {
                let fixture = registry.fixture();
                serialized(non_admin(server, &fixture.registry, &fixture.public_ids))
            }
            Input::NonAdminList { registry, servers } => {
                let fixture = registry.fixture();
                serialized(non_admin_list(
                    servers,
                    &fixture.registry,
                    &fixture.public_ids,
                ))
            }
            Input::VirtualKey { registry, server } => {
                let fixture = registry.fixture();
                serialized(virtual_key(server, &fixture.registry, &fixture.public_ids))
            }
            Input::VirtualKeyList { registry, servers } => {
                let fixture = registry.fixture();
                serialized(virtual_key_list(
                    servers,
                    &fixture.registry,
                    &fixture.public_ids,
                ))
            }
            Input::FullAdmin { auth } => full_admin(auth),
            Input::CredentialTarget {
                auth,
                requested_user_id,
            } => credential_target(auth, requested_user_id.as_deref()),
            Input::HasNonAdminCredentials { credentials } => Ok(json!({
                "has_non_admin_config_credentials": has_config_credentials(credentials.as_ref()),
            })),
            Input::InheritCredentials { registry, payload } => inherit_answer(registry, payload),
        }
    }
}

fn redacted_list(registry: &RegistryInput, servers: &[McpServerTable]) -> HelperResult {
    let fixture = registry.fixture();
    let redacted = redact_mcp_credentials_list(servers, &fixture.registry, &fixture.public_ids)
        .or_config_error()?;
    serde_json::to_value(redacted).or_config_error()
}

fn inherit_answer(registry: &RegistryInput, payload: &NewMCPServerRequest) -> HelperResult {
    let fixture = registry.fixture();
    let inherited = inherit_credentials(payload, &fixture.registry);
    serde_json::to_value(inherited.credentials).or_config_error()
}

fn full_admin(auth: &BTreeMap<String, JsonData>) -> HelperResult {
    let caller = UserAPIKeyAuth::construct(AuthInput::Dict(auth.clone())).or_value_error()?;
    Ok(json!({
        "full_admin": user_is_full_admin(&caller),
    }))
}

fn credential_target(
    auth: &BTreeMap<String, JsonData>,
    requested_user_id: Option<&str>,
) -> HelperResult {
    let caller = UserAPIKeyAuth::construct(AuthInput::Dict(auth.clone())).or_value_error()?;
    match target_user_id(&caller, requested_user_id) {
        Ok(user_id) => Ok(json!({ "user_id": user_id })),
        Err(error) => Ok(json!({ "status": error.status_code().as_u16() })),
    }
}

// ast-grep-ignore: no-one-line-functions - every sanitizer arm ends with the same view-to-JSON step
fn serialized<T>(view: Result<T, CredentialViewError>) -> HelperResult
where
    T: Serialize,
{
    serde_json::to_value(view.or_config_error()?).or_config_error()
}
