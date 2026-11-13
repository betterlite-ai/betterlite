mod tests;

use betterlite_mcp::credential_contracts::auth_spec::AuthResolution;
use betterlite_mcp::request_context::McpContext;

const MAX_REPORTED_SERVERS: usize = 32;
pub const AUTH_RESOLUTION_HEADER: &str = "x-mcp-debug-auth-resolution";
pub const AUTH_RESOLUTIONS_HEADER: &str = "x-mcp-debug-auth-resolutions";
pub const AUTH_RESOLUTIONS_TRUNCATED: &str = "x-mcp-debug-auth-resolutions-truncated";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MCPAuthDiagnostics {
    outcomes: Vec<(String, AuthResolution)>,
}

impl MCPAuthDiagnostics {
    pub fn record(&mut self, server_id: &str, resolution: AuthResolution) {
        self.outcomes
            .retain(|(existing, _resolution)| existing != server_id);
        self.outcomes.push((server_id.to_owned(), resolution));
    }

    #[must_use]
    pub const fn resolution(&self) -> AuthResolution {
        match self.outcomes.as_slice() {
            [] => AuthResolution::Unresolved,
            [(_, resolution)] => *resolution,
            _ => AuthResolution::Multiple,
        }
    }

    #[must_use]
    pub fn headers(&self) -> Vec<(String, String)> {
        if self.outcomes.len() <= 1 {
            return vec![(
                AUTH_RESOLUTION_HEADER.to_owned(),
                self.resolution().to_string(),
            )];
        }
        let mut headers = vec![
            (
                AUTH_RESOLUTION_HEADER.to_owned(),
                AuthResolution::Multiple.to_string(),
            ),
            (AUTH_RESOLUTIONS_HEADER.to_owned(), self.resolution_map()),
        ];
        if self.outcomes.len() > MAX_REPORTED_SERVERS {
            headers.push((AUTH_RESOLUTIONS_TRUNCATED.to_owned(), "true".to_owned()));
        }
        headers
    }

    fn resolution_map(&self) -> String {
        let mut map = String::from("{");
        for (index, (server_id, resolution)) in
            self.outcomes.iter().take(MAX_REPORTED_SERVERS).enumerate()
        {
            if index > 0 {
                map.push(',');
            }
            map.push_str(
                &serde_json::to_string(server_id).unwrap_or_else(|_error| "\"\"".to_owned()),
            );
            map.push_str(":\"");
            map.push_str(&resolution.to_string());
            map.push('"');
        }
        map.push('}');
        map
    }
}

pub fn record_auth_resolution(
    context: &mut McpContext,
    server_id: &str,
    resolution: AuthResolution,
) {
    if let Some(diagnostics) = context.request_scope.get_mut::<MCPAuthDiagnostics>() {
        diagnostics.record(server_id, resolution);
    }
}
