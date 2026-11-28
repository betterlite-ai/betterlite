use std::time::Duration;

use betterlite_db::managed_resource_rows::mcp::McpStatus;
use betterlite_db::mcp_stored_contracts::MCPTransport;
use url::Url;

use super::ports::{
    AllowedServers, Clock, EnvVarCheck, ProbeClient, ProbeError, SessionProbe, SpecLoader,
};
use super::service::HealthService;
use crate::server_record::MCPServer;

pub const ACCEPT: &str = "text/event-stream, application/json";

const NEEDS_HTTP_TRANSPORT: &str = "Server reachability requires an HTTP or SSE URL";
const NEEDS_CLEAN_URL: &str =
    "Server reachability requires an HTTP URL without embedded credentials";
const CANCELLED: &str = "Reachability check was cancelled";

impl<P, S, E, A, C, L> HealthService<P, S, E, A, C, L>
where
    P: ProbeClient,
    S: SessionProbe,
    E: EnvVarCheck,
    A: AllowedServers,
    C: Clock,
    L: SpecLoader,
{
    #[must_use]
    pub async fn reachability(
        &self,
        server: &MCPServer,
        timeout: Duration,
    ) -> (McpStatus, Option<String>) {
        let url = match reachable_url(server) {
            Ok(url) => url,
            Err(message) => return (McpStatus::Unknown, Some(message.to_owned())),
        };
        let probe = self
            .deps
            .probe
            .get_stream_status(url.as_str(), ACCEPT, timeout);
        let Some(outcome) = self.deps.cancel.run_until_cancelled(probe).await else {
            return (McpStatus::Unknown, Some(CANCELLED.to_owned()));
        };
        match outcome {
            Ok(()) => (McpStatus::Reachable, None),
            Err(ProbeError::Timeout) => (
                McpStatus::Unhealthy,
                Some(format!(
                    "Reachability check timed out after {:?} seconds",
                    timeout.as_secs_f64()
                )),
            ),
            Err(ProbeError::Failed(kind)) => (
                McpStatus::Unhealthy,
                Some(format!("Reachability check failed ({kind})")),
            ),
        }
    }
}

// LiteLLM parses the url with httpx.URL and refuses a scheme, a host or userinfo it cannot probe
fn reachable_url(server: &MCPServer) -> Result<Url, &'static str> {
    let Some(raw_url) = server.fields.url.as_deref().filter(|url| !url.is_empty()) else {
        return Err(NEEDS_HTTP_TRANSPORT);
    };
    match server.fields.transport {
        MCPTransport::Http | MCPTransport::Sse => {}
        MCPTransport::Stdio => return Err(NEEDS_HTTP_TRANSPORT),
    }
    let parsed = Url::parse(raw_url).map_err(|_error| NEEDS_CLEAN_URL)?;
    let clean = matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none();
    if !clean {
        return Err(NEEDS_CLEAN_URL);
    }
    Ok(parsed)
}
