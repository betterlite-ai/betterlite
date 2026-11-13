pub mod auth_diagnostics;
pub mod headers;
pub mod preview;
mod tests;

use std::collections::BTreeMap;

use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_mcp::request_headers::client_headers::get_litellm_api_key;
use betterlite_mcp::request_headers::raw_headers::RawHeaders;
use betterlite_redact::structured::SensitiveDataMasker;
use secrecy::SecretString;

use self::auth_diagnostics::MCPAuthDiagnostics;
use self::headers::{DebugHeaderInputs, DebugServer, build_debug_headers};

pub const MCP_DEBUG_REQUEST_HEADER: &str = "x-litellm-mcp-debug";
pub const RESPONSE_HEADER_PREFIX: &str = "x-mcp-debug";

#[must_use]
pub fn is_debug_enabled(headers: &RawHeaders) -> bool {
    let lowered = headers
        .lookup(MCP_DEBUG_REQUEST_HEADER)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if lowered == "true" {
        return true;
    }
    if lowered == "1" {
        return true;
    }
    lowered == "yes"
}

#[must_use]
pub fn mask_secret(value: Option<&str>) -> String {
    let Some(text) = value.filter(|text| !text.is_empty()) else {
        return "(none)".to_owned();
    };
    let defaults = SensitiveDataMasker::default();
    let masker = SensitiveDataMasker {
        sensitive_patterns: defaults.sensitive_patterns,
        non_sensitive_overrides: defaults.non_sensitive_overrides,
        visible_prefix: 6,
        visible_suffix: 4,
        mask_char: defaults.mask_char,
        mask_short_values: defaults.mask_short_values,
        max_work: defaults.max_work,
    };
    masker.mask_value(text)
}

pub struct DebugSendHeaders<'diagnostics> {
    pub debug_headers: Vec<(String, String)>,
    // Some only for a POST: any other method sends the headers as they were built
    pub diagnostics: Option<&'diagnostics MCPAuthDiagnostics>,
}

impl DebugSendHeaders<'_> {
    #[must_use]
    pub fn headers(&self) -> Vec<(String, String)> {
        let Some(diagnostics) = self.diagnostics else {
            return self.debug_headers.clone();
        };
        let mut headers = self.debug_headers.clone();
        for (name, value) in diagnostics.headers() {
            match headers
                .iter_mut()
                .find(|(existing, _value)| *existing == name)
            {
                Some(entry) => entry.1 = value,
                None => headers.push((name, value)),
            }
        }
        headers
    }
}

#[must_use]
pub fn maybe_build_debug_headers(
    raw_headers: Option<&RawHeaders>,
    servers: &[DebugServer<'_>],
    oauth2_headers: Option<&BTreeMap<String, SecretString>>,
) -> Option<Vec<(String, String)>> {
    let enabled_headers = raw_headers.filter(|headers| is_debug_enabled(headers))?;
    let first_server = servers.first();
    let api_key = get_litellm_api_key(enabled_headers);
    Some(build_debug_headers(&DebugHeaderInputs {
        inbound_headers: enabled_headers,
        oauth2_headers,
        litellm_api_key: api_key.value.as_ref(),
        auth_resolution: AuthResolution::Unresolved,
        server_url: first_server.and_then(|server| server.url),
        server_auth_type: first_server.and_then(|server| server.auth_type),
    }))
}
