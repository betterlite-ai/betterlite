use betterlite_db::mcp_stored_contracts::MCPAuth;
use strum::EnumString;

use crate::server_payload::request::NewMCPServerRequest;

#[derive(Clone, Copy, Debug, Eq, EnumString, PartialEq)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum NewServerField {
    ServerId,
    ServerName,
    Alias,
    Description,
    Transport,
    AuthType,
    Credentials,
    Url,
    SpecPath,
    McpInfo,
    McpAccessGroups,
    AllowedTools,
    ToolNameToDisplayName,
    ToolNameToDescription,
    ExtraHeaders,
    StaticHeaders,
    EnvVars,
    Instructions,
    Command,
    Args,
    Env,
    Issuer,
    AuthorizationUrl,
    TokenUrl,
    RegistrationUrl,
    Oauth2Flow,
    TokenExchangeEndpoint,
    Audience,
    SubjectTokenType,
    TokenExchangeProfile,
    AllowAllKeys,
    AvailableOnPublicInternet,
    DelegateAuthToUpstream,
    OauthPassthrough,
    DcrBridge,
    PerServerOauthDiscovery,
    IsByok,
    ByokDescription,
    ByokApiKeyHelpUrl,
    SourceUrl,
    Timeout,
    MaxConcurrentRequests,
    ApprovalStatus,
    SubmittedBy,
    SubmittedAt,
}

fn non_empty(value: Option<&str>) -> bool {
    let Some(text) = value else {
        return false;
    };
    !text.is_empty()
}

fn non_empty_list<T>(values: Option<&[T]>) -> bool {
    let Some(values) = values else {
        return false;
    };
    !values.is_empty()
}

pub(super) fn present(payload: &NewMCPServerRequest, field: NewServerField) -> bool {
    match field {
        NewServerField::ServerId => non_empty(payload.server_id.as_deref()),
        NewServerField::ServerName => non_empty(payload.server_name.as_deref()),
        NewServerField::Alias => non_empty(payload.alias.as_deref()),
        NewServerField::Description => non_empty(payload.description.as_deref()),
        NewServerField::Transport => true,
        NewServerField::AuthType => payload.auth_type.is_some_and(|auth| auth != MCPAuth::None),
        NewServerField::Credentials => payload.credentials.is_some(),
        NewServerField::Url => non_empty(payload.url.as_deref()),
        NewServerField::SpecPath => non_empty(payload.spec_path.as_deref()),
        NewServerField::McpInfo => payload.mcp_info.is_some(),
        NewServerField::McpAccessGroups => !payload.mcp_access_groups.is_empty(),
        NewServerField::AllowedTools => non_empty_list(payload.allowed_tools.as_deref()),
        NewServerField::ToolNameToDisplayName => payload.tool_name_to_display_name.is_some(),
        NewServerField::ToolNameToDescription => payload.tool_name_to_description.is_some(),
        NewServerField::ExtraHeaders => non_empty_list(payload.extra_headers.as_deref()),
        NewServerField::StaticHeaders => payload.static_headers.is_some(),
        NewServerField::EnvVars => non_empty_list(payload.env_vars.as_deref()),
        NewServerField::Instructions => non_empty(payload.instructions.as_deref()),
        NewServerField::Command => non_empty(payload.command.as_deref()),
        NewServerField::Args => !payload.args.is_empty(),
        // an empty dict counts as present, unlike an empty list
        NewServerField::Env => true,
        NewServerField::Issuer => non_empty(payload.issuer.as_deref()),
        NewServerField::AuthorizationUrl => non_empty(payload.authorization_url.as_deref()),
        NewServerField::TokenUrl => non_empty(payload.token_url.as_deref()),
        NewServerField::RegistrationUrl => non_empty(payload.registration_url.as_deref()),
        NewServerField::Oauth2Flow => payload.oauth2_flow.is_some(),
        NewServerField::TokenExchangeEndpoint => {
            non_empty(payload.token_exchange_endpoint.as_deref())
        }
        NewServerField::Audience => non_empty(payload.audience.as_deref()),
        NewServerField::SubjectTokenType => non_empty(payload.subject_token_type.as_deref()),
        NewServerField::TokenExchangeProfile => {
            non_empty(payload.token_exchange_profile.as_deref())
        }
        NewServerField::AllowAllKeys => true,
        NewServerField::AvailableOnPublicInternet => true,
        NewServerField::DelegateAuthToUpstream => true,
        NewServerField::OauthPassthrough => true,
        NewServerField::DcrBridge => payload.dcr_bridge.is_some(),
        NewServerField::PerServerOauthDiscovery => true,
        NewServerField::IsByok => true,
        NewServerField::ByokDescription => !payload.byok_description.is_empty(),
        NewServerField::ByokApiKeyHelpUrl => non_empty(payload.byok_api_key_help_url.as_deref()),
        NewServerField::SourceUrl => non_empty(payload.source_url.as_deref()),
        NewServerField::Timeout => payload.timeout.is_some(),
        NewServerField::MaxConcurrentRequests => payload.max_concurrent_requests.is_some(),
        NewServerField::ApprovalStatus => non_empty(payload.approval_status.as_deref()),
        NewServerField::SubmittedBy => non_empty(payload.submitted_by.as_deref()),
        NewServerField::SubmittedAt => non_empty(payload.submitted_at.as_deref()),
    }
}
