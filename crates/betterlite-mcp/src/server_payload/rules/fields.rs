use crate::server_payload::request::NewMCPServerRequest;
use betterlite_db::mcp_stored_contracts::MCPAuth;
use strum::EnumString;
macro_rules! new_server_fields {
    (
        always: $($always:ident),* $(,)? ;
        text: $($text_variant:ident($text_field:ident)),* $(,)? ;
        auth: $($auth_variant:ident($auth_field:ident)),* $(,)? ;
        some: $($some_variant:ident($some_field:ident)),* $(,)? ;
        list: $($list_variant:ident($list_field:ident)),* $(,)? ;
        optional_list: $($optional_variant:ident($optional_field:ident)),* $(,)? ;
    ) => {
        #[derive(Clone, Copy, Debug, Eq, EnumString, PartialEq)]
        #[strum(serialize_all = "snake_case")]
        pub(crate) enum NewServerField {
            $($always,)*
            $($text_variant,)*
            $($auth_variant,)*
            $($some_variant,)*
            $($list_variant,)*
            $($optional_variant,)*
        }
        pub(super) fn present(payload: &NewMCPServerRequest, field: NewServerField) -> bool {
            match field {
                $(NewServerField::$always)|* => true,
                $(NewServerField::$text_variant => {
                    payload.$text_field.as_deref().is_some_and(|text| !text.is_empty())
                })*
                $(NewServerField::$auth_variant => {
                    payload.$auth_field.is_some_and(|auth| auth != MCPAuth::None)
                })*
                $(NewServerField::$some_variant => payload.$some_field.is_some(),)*
                $(NewServerField::$list_variant => !payload.$list_field.is_empty(),)*
                $(NewServerField::$optional_variant => {
                    payload.$optional_field.as_deref().is_some_and(|values| !values.is_empty())
                })*
            }
        }
    };
}
// an empty dict counts as present, unlike an empty list
new_server_fields! {
    always: Transport, Env, AllowAllKeys, AvailableOnPublicInternet, DelegateAuthToUpstream,
        OauthPassthrough, PerServerOauthDiscovery, IsByok;
    text: ServerId(server_id), ServerName(server_name), Alias(alias), Description(description),
        Url(url), SpecPath(spec_path), Instructions(instructions), Command(command), Issuer(issuer),
        AuthorizationUrl(authorization_url), TokenUrl(token_url), RegistrationUrl(registration_url),
        TokenExchangeEndpoint(token_exchange_endpoint), Audience(audience),
        SubjectTokenType(subject_token_type), TokenExchangeProfile(token_exchange_profile),
        ByokApiKeyHelpUrl(byok_api_key_help_url), SourceUrl(source_url),
        ApprovalStatus(approval_status), SubmittedBy(submitted_by), SubmittedAt(submitted_at);
    auth: AuthType(auth_type);
    some: Credentials(credentials), McpInfo(mcp_info),
        ToolNameToDisplayName(tool_name_to_display_name),
        ToolNameToDescription(tool_name_to_description), StaticHeaders(static_headers),
        Oauth2Flow(oauth2_flow), DcrBridge(dcr_bridge), Timeout(timeout),
        MaxConcurrentRequests(max_concurrent_requests);
    list: McpAccessGroups(mcp_access_groups), Args(args), ByokDescription(byok_description);
    optional_list: AllowedTools(allowed_tools), ExtraHeaders(extra_headers), EnvVars(env_vars);
}
