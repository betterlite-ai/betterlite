use std::time::Duration;

use betterlite_db::managed_resource_rows::mcp::McpStatus;
use betterlite_db::mcp_stored_contracts::MCPTransport;
use thiserror::Error;
use url::Url;

use super::ports::{
    AllowedServers, Clock, EnvVarCheck, ProbeClient, ProbeError, SessionProbe, SpecLoader,
};
use super::service::HealthService;
use crate::server_record::MCPServer;

pub const ACCEPT: &str = "text/event-stream, application/json";

#[derive(Debug, Error)]
enum UrlRefusal {
    #[error("Server reachability requires an HTTP or SSE URL")]
    Transport,
    #[error("Server reachability requires an HTTP URL without embedded credentials")]
    Credentials,
    #[error("Server reachability requires an HTTP URL without embedded credentials")]
    Unparsable(#[from] url::ParseError),
}

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
            Err(error) => return (McpStatus::Unknown, Some(error.to_string())),
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
fn reachable_url(server: &MCPServer) -> Result<Url, UrlRefusal> {
    let Some(raw_url) = server.fields.url.as_deref().filter(|url| !url.is_empty()) else {
        return Err(UrlRefusal::Transport);
    };
    match server.fields.transport {
        MCPTransport::Http | MCPTransport::Sse => {}
        MCPTransport::Stdio => return Err(UrlRefusal::Transport),
    }
    let parsed = Url::parse(raw_url)?;
    let clean = matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none();
    if !clean {
        return Err(UrlRefusal::Credentials);
    }
    Ok(parsed)
}
