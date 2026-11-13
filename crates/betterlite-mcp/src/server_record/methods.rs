use betterlite_db::mcp_stored_contracts::MCPAuth;
use betterlite_db::mcp_stored_contracts::MCPUpstreamProtocol;
use betterlite_db::mcp_stored_contracts::pinned::MCPInfo;
use serde::Deserialize;

use super::binding::BindingMode;
use super::{MCPServer, OAuth2Flow, ServerFields, ServerRecordError};
use crate::credential_contracts::header_carrier::validate_header_name;

const AUTH_HEADER_NAMES: [&str; 4] = ["authorization", "x-api-key", "api-key", "apikey"];

fn is_auth_header(header: &str) -> bool {
    AUTH_HEADER_NAMES
        .iter()
        .any(|name| header.eq_ignore_ascii_case(name))
}

#[derive(Deserialize)]
struct ProtocolMetadata {
    #[serde(default)]
    protocol_version: Option<MCPUpstreamProtocol>,
}

fn protocol_version_from_info(info: &MCPInfo) -> Result<MCPUpstreamProtocol, ServerRecordError> {
    let encoded = serde_json::to_string(info)?;
    let metadata: ProtocolMetadata = serde_json::from_str(&encoded)?;
    Ok(metadata
        .protocol_version
        .unwrap_or(MCPUpstreamProtocol::Auto))
}

impl ServerFields {
    fn normalize_token_header(&mut self) -> Result<(), ServerRecordError> {
        let Some(raw) = self.upstream_token_header.take() else {
            return Ok(());
        };
        if raw.trim().is_empty() {
            return Ok(());
        }
        self.upstream_token_header = Some(validate_header_name(&raw)?);
        Ok(())
    }

    fn resolve_protocol_version(&mut self) -> Result<(), ServerRecordError> {
        if self.protocol_version.is_some() {
            return Ok(());
        }
        let resolved = match &self.mcp_info {
            Some(info) => protocol_version_from_info(info)?,
            None => MCPUpstreamProtocol::Auto,
        };
        self.protocol_version = Some(resolved);
        Ok(())
    }
}

impl MCPServer {
    pub fn checked(mut self) -> Result<Self, ServerRecordError> {
        self.fields.normalize_token_header()?;
        self.fields.resolve_protocol_version()?;
        self.validate_identity_binding()?;
        Ok(self)
    }

    fn validate_identity_binding(&self) -> Result<(), ServerRecordError> {
        let Some(binding) = &self.fields.oauth_identity_binding else {
            return Ok(());
        };
        if binding.mode == BindingMode::Disabled {
            return Ok(());
        }
        if !self.needs_user_oauth_token() || self.fields.delegate_auth_to_upstream {
            return Err(ServerRecordError::Binding(
                "oauth_identity_binding requires gateway-managed per-user OAuth2 credentials"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn effective_authorization_url(&self) -> Option<&str> {
        self.fields
            .authorization_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .or(self.fields.configured_authorization_url.as_deref())
    }

    #[must_use]
    pub fn effective_token_url(&self) -> Option<&str> {
        self.fields
            .token_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .or(self.fields.configured_token_url.as_deref())
    }

    #[must_use]
    pub fn effective_registration_url(&self) -> Option<&str> {
        self.fields
            .registration_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .or(self.fields.configured_registration_url.as_deref())
    }

    #[must_use]
    pub const fn has_client_credentials(&self) -> bool {
        match self.fields.oauth2_flow {
            Some(OAuth2Flow::ClientCredentials) => true,
            Some(OAuth2Flow::AuthorizationCode) | None => false,
        }
    }

    #[must_use]
    pub const fn needs_user_oauth_token(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::Oauth2) => !self.has_client_credentials(),
            Some(_) | None => false,
        }
    }

    #[must_use]
    pub const fn is_gateway_managed_oauth2(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::Oauth2) => !self.fields.delegate_auth_to_upstream,
            Some(_) | None => false,
        }
    }

    #[must_use]
    pub const fn uses_per_server_oauth_relay(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::Oauth2) => {
                self.fields.per_server_oauth_discovery && !self.has_client_credentials()
            }
            Some(_) | None => false,
        }
    }

    // LiteLLM names this property advertises_gateway_authorization_server, and the task keeps its names
    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn advertises_gateway_authorization_server(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::Oauth2) => {
                self.is_gateway_managed_oauth2() && !self.uses_per_server_oauth_relay()
            }
            Some(
                MCPAuth::Oauth2TokenExchange
                | MCPAuth::Oauth2IdJag
                | MCPAuth::TruePassthrough
                | MCPAuth::OauthDelegate,
            ) => false,
            Some(
                MCPAuth::None
                | MCPAuth::ApiKey
                | MCPAuth::BearerToken
                | MCPAuth::Basic
                | MCPAuth::Authorization
                | MCPAuth::Token
                | MCPAuth::AwsSigv4,
            )
            | None => !self.has_auth_extra_header(),
        }
    }

    #[must_use]
    pub const fn is_true_passthrough(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::TruePassthrough) => true,
            Some(_) | None => false,
        }
    }

    #[must_use]
    pub const fn is_oauth_delegate(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::OauthDelegate) => true,
            Some(_) | None => false,
        }
    }

    #[must_use]
    pub const fn is_client_forwarded_token(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::TruePassthrough | MCPAuth::OauthDelegate) => true,
            Some(_) | None => false,
        }
    }

    #[must_use]
    pub const fn is_dcr_bridge(&self) -> bool {
        match self.fields.dcr_bridge {
            Some(true) => self.is_client_forwarded_token(),
            Some(false) | None => false,
        }
    }

    #[must_use]
    pub fn requires_per_user_auth(&self) -> bool {
        match self.fields.auth_type {
            Some(MCPAuth::None) => self.has_auth_extra_header(),
            Some(auth) => {
                (auth == MCPAuth::Oauth2 && !self.has_client_credentials())
                    || auth == MCPAuth::TruePassthrough
                    || auth == MCPAuth::OauthDelegate
            }
            None => false,
        }
    }

    #[must_use]
    pub fn is_oauth_passthrough(&self) -> bool {
        match self.fields.auth_type {
            None | Some(MCPAuth::None) => {
                self.fields.oauth_passthrough && self.has_authorization_header()
            }
            Some(_) => false,
        }
    }

    fn has_auth_extra_header(&self) -> bool {
        self.fields
            .extra_headers
            .as_ref()
            .is_some_and(|headers| headers.iter().any(|header| is_auth_header(header.as_str())))
    }

    fn has_authorization_header(&self) -> bool {
        self.fields.extra_headers.as_ref().is_some_and(|headers| {
            headers
                .iter()
                .any(|header| header.eq_ignore_ascii_case("authorization"))
        })
    }
}
