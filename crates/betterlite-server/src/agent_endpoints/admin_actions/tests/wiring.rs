use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_auth::agent_policy::AgentResponse as PolicyAgent;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig, AgentKillSwitchMethod};
use secrecy::ExposeSecret as _;
use thiserror::Error;

use super::support::{ADMIN, Fixture, caller, kill_switch_path, post};
use crate::agent_endpoints::admin_actions::ports::{DeliveringTrigger, KillSwitchTrigger};
use crate::kill_switch_management::client::{
    KillSwitchHttpClient, KillSwitchResponse, KillSwitchSendOptions,
};
use crate::kill_switch_management::request::KillSwitchRequest;

const POLICY_AGENT: &str = r#"{"agent_id":"agent-1","agent_name":"Agent One","agent_card_params":{},"kill_switch":{"url":"https://agent.example/stop","method":"PUT","headers":{"x-trace":"on"},"query_params":{},"body":null,"auth":{"type":"bearer","token":"sk-live"}}}"#;

#[tokio::test]
async fn the_registry_adapter_keeps_the_stored_kill_switch() {
    let fixture = Fixture::new();
    let registry = AgentRegistry::default();
    let agent: PolicyAgent = serde_json::from_str(POLICY_AGENT).unwrap();
    registry.register_agent(agent);
    let mut state = fixture.state();
    state.registry = Arc::new(registry);
    let reply = post(state, &kill_switch_path("agent-1"), &caller(ADMIN)).await;
    assert_eq!(reply.status, StatusCode::OK);
    let fired = fixture.trigger.fired();
    assert_eq!(fired[0].1.url, "https://agent.example/stop");
    assert_eq!(fired[0].1.method, AgentKillSwitchMethod::Put);
    assert_eq!(fired[0].1.headers["x-trace"], "on");
    let Some(AgentKillSwitchAuth::Bearer { token }) = fired[0].1.auth.as_ref() else {
        panic!("the bearer auth survives the serde round trip");
    };
    assert_eq!(token.expose_secret(), "sk-live");
}

#[derive(Debug, Error)]
#[error("the client cannot build requests")]
struct ClientFailure;

struct UnbuildableClient;

struct NoResponse;

impl KillSwitchHttpClient for UnbuildableClient {
    type Error = ClientFailure;
    type Request = ();
    type Response = NoResponse;

    fn build_request(
        &self,
        _request: &KillSwitchRequest,
        _timeout: Duration,
    ) -> Result<(), ClientFailure> {
        Err(ClientFailure)
    }

    fn send(
        &self,
        _request: (),
        _options: KillSwitchSendOptions,
    ) -> impl Future<Output = Result<NoResponse, ClientFailure>> + Send {
        ready(Err(ClientFailure))
    }
}

impl KillSwitchResponse for NoResponse {
    type Error = ClientFailure;

    fn status(&self) -> u16 {
        0
    }

    fn next_text_chunk(
        &mut self,
    ) -> impl Future<Output = Option<Result<String, ClientFailure>>> + Send {
        ready(None)
    }

    fn close(&mut self) -> impl Future<Output = Result<(), ClientFailure>> + Send {
        ready(Ok(()))
    }
}

fn switch_config() -> AgentKillSwitchConfig {
    AgentKillSwitchConfig {
        url: String::from("https://agent.example/stop"),
        method: AgentKillSwitchMethod::Post,
        headers: BTreeMap::new(),
        query_params: BTreeMap::new(),
        body: None,
        auth: None,
    }
}

#[tokio::test]
async fn the_delivering_trigger_reports_the_client_failure() {
    let trigger = DeliveringTrigger {
        client: UnbuildableClient,
    };
    let result = trigger.fire("agent-1", &switch_config()).await;
    assert_eq!(result.agent_id, "agent-1");
    assert_eq!(result.url, "https://agent.example/stop");
    assert_eq!(result.status_code, None);
    assert_eq!(
        result.error.as_deref(),
        Some("the client cannot build requests")
    );
}
