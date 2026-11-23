use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::json;

use betterlite_mcp::credential_contracts::errors::CredError;
use betterlite_mcp::credentials::McpAuthType;
use betterlite_mcp::static_credential::validate_static_credential;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    auth_type: String,
    headers: BTreeMap<String, String>,
    upstream_token_header: Option<String>,
    static_header_names: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let auth_type = auth_mode(&input.auth_type).or_value_error()?;
        let result = validate_static_credential(
            auth_type,
            &input.headers,
            input.upstream_token_header.as_deref(),
            &input.static_header_names,
        );
        Ok(match result {
            Ok(()) => json!({"ok": true, "error": null}),
            Err(error) => json!({"ok": false, "error": error_tag(&error)}),
        })
    }
}

fn auth_mode(raw: &str) -> Result<McpAuthType, String> {
    match raw {
        "none" => Ok(McpAuthType::None),
        "api_key" => Ok(McpAuthType::ApiKey),
        "bearer_token" => Ok(McpAuthType::BearerToken),
        "basic" => Ok(McpAuthType::Basic),
        "authorization" => Ok(McpAuthType::Authorization),
        "oauth2" => Ok(McpAuthType::Oauth2),
        "token" => Ok(McpAuthType::Token),
        "oauth2_token_exchange" => Ok(McpAuthType::Oauth2TokenExchange),
        other => Err(format!("unknown auth_type {other}")),
    }
}

fn error_tag(error: &CredError) -> &'static str {
    match error {
        CredError::Unauthorized(_) => "unauthorized",
        CredError::Misconfigured(_) => "misconfigured",
        CredError::UrlCredentialsNotAllowed => "url_credentials_not_allowed",
        CredError::UpstreamUnavailable(_) => "upstream_unavailable",
        CredError::UnsupportedMode(_) => "unsupported_mode",
        CredError::PreconditionRequired(_) => "precondition_required",
        CredError::NotImplemented(_) => "not_implemented",
    }
}
