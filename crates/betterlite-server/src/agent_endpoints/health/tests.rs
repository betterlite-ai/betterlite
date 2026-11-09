#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::agent_search::Settings as AgentSearchSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_core::agents::AgentResponse;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use rstest::rstest;
use serde_json::from_str;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::{
    AgentHealthError, AgentHealthResult, AgentHealthTransport, PooledHealthTransport,
    check_agent_url_health, filter_healthy_agents, probe_agents,
};

const FIVE_SECONDS: Duration = Duration::from_secs(5);

fn agent(agent_id: &str, url: Option<&str>) -> AgentResponse {
    let card = url.map_or_else(
        || "{}".to_owned(),
        |target| format!(r#"{{"url":"{target}"}}"#),
    );
    let source = format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_id}","agent_card_params":{card}}}"#
    );
    from_str(&source).unwrap()
}

fn settings(probe_seconds: f64, gather_seconds: f64) -> AgentSearchSettings {
    AgentSearchSettings {
        embedding_model: None,
        health_check_timeout_seconds: probe_seconds,
        health_gather_timeout_seconds: gather_seconds,
    }
}

struct FakeTransport {
    status: u16,
    failure: Option<String>,
    calls: Mutex<Vec<String>>,
}

impl FakeTransport {
    fn answering(status: u16) -> Self {
        Self {
            status,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            status: 0,
            failure: Some(message.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn probed(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl AgentHealthTransport for FakeTransport {
    async fn get_status(&self, url: &str, _timeout: Duration) -> Result<u16, AgentHealthError> {
        self.calls.lock().unwrap().push(url.to_owned());
        self.failure.as_ref().map_or_else(
            || Ok(self.status),
            |message| {
                Err(AgentHealthError::Unreachable {
                    message: message.clone(),
                })
            },
        )
    }
}

struct SlowTransport;

#[async_trait]
impl AgentHealthTransport for SlowTransport {
    async fn get_status(&self, _url: &str, _timeout: Duration) -> Result<u16, AgentHealthError> {
        sleep(Duration::from_secs(30)).await;
        Ok(200)
    }
}

struct BarrierTransport {
    barrier: Arc<Barrier>,
}

#[async_trait]
impl AgentHealthTransport for BarrierTransport {
    async fn get_status(&self, _url: &str, _timeout: Duration) -> Result<u16, AgentHealthError> {
        self.barrier.wait().await;
        Ok(200)
    }
}

#[rstest]
#[case::absent_url(None)]
#[case::empty_url(Some(""))]
#[tokio::test]
async fn url_less_agents_are_healthy_without_a_request(#[case] url: Option<&str>) {
    let transport = FakeTransport::answering(200);
    let result = check_agent_url_health(&agent("a1", url), &transport, FIVE_SECONDS).await;
    assert_eq!(
        result,
        AgentHealthResult {
            agent_id: "a1".to_owned(),
            healthy: true,
            error: None,
        }
    );
    assert!(transport.probed().is_empty());
}

#[rstest]
#[case::internal_error(500)]
#[case::service_unavailable(503)]
#[tokio::test]
async fn server_error_statuses_are_unhealthy_with_the_http_status(#[case] status: u16) {
    let transport = FakeTransport::answering(status);
    let result = check_agent_url_health(
        &agent("a1", Some("https://agent.example")),
        &transport,
        FIVE_SECONDS,
    )
    .await;
    assert_eq!(
        result,
        AgentHealthResult {
            agent_id: "a1".to_owned(),
            healthy: false,
            error: Some(format!("HTTP {status}")),
        }
    );
}

#[rstest]
#[case::not_found(404)]
#[case::unauthorized(401)]
#[case::redirect(302)]
#[tokio::test]
async fn statuses_below_server_error_are_healthy(#[case] status: u16) {
    let transport = FakeTransport::answering(status);
    let result = check_agent_url_health(
        &agent("a1", Some("https://agent.example")),
        &transport,
        FIVE_SECONDS,
    )
    .await;
    assert_eq!(
        result,
        AgentHealthResult {
            agent_id: "a1".to_owned(),
            healthy: true,
            error: None,
        }
    );
}

#[tokio::test]
async fn transport_errors_are_unhealthy_with_their_message() {
    let transport = FakeTransport::failing("connection refused");
    let result = check_agent_url_health(
        &agent("a1", Some("https://agent.example")),
        &transport,
        FIVE_SECONDS,
    )
    .await;
    assert_eq!(
        result,
        AgentHealthResult {
            agent_id: "a1".to_owned(),
            healthy: false,
            error: Some("connection refused".to_owned()),
        }
    );
}

#[tokio::test]
async fn filter_returns_healthy_agents_then_url_less_agents_in_input_order() {
    let transport = FakeTransport::failing("unreachable");
    let agents = vec![
        agent("a1", None),
        agent("a2", Some("https://agent.example/two")),
        agent("a3", None),
        agent("a4", Some("")),
    ];
    let healthy = filter_healthy_agents(agents, &transport, &settings(5.0, 30.0)).await;
    assert_eq!(
        healthy
            .iter()
            .map(|agent| agent.agent_id.as_str())
            .collect::<Vec<_>>(),
        ["a1", "a3", "a4"]
    );
    assert_eq!(transport.probed(), ["https://agent.example/two"]);
}

#[tokio::test]
async fn filter_probes_agents_with_a_url_concurrently() {
    let transport = BarrierTransport {
        barrier: Arc::new(Barrier::new(2)),
    };
    let agents = vec![
        agent("a1", Some("https://agent.example/one")),
        agent("a2", Some("https://agent.example/two")),
    ];
    let healthy = filter_healthy_agents(agents, &transport, &settings(5.0, 60.0)).await;
    assert_eq!(
        healthy
            .iter()
            .map(|agent| agent.agent_id.as_str())
            .collect::<Vec<_>>(),
        ["a1", "a2"]
    );
}

#[tokio::test]
async fn probe_results_mark_every_agent_unhealthy_when_the_gather_timeout_fires() {
    let agents = vec![
        agent("a1", Some("https://agent.example/one")),
        agent("a2", Some("https://agent.example/two")),
    ];
    let results = probe_agents(&agents, &SlowTransport, &settings(5.0, 0.05)).await;
    assert_eq!(results.len(), 2);
    assert!(!results[0].healthy);
    assert_eq!(results[0].error.as_deref(), Some("Health check timed out"));
    assert!(!results[1].healthy);
    assert_eq!(results[1].error.as_deref(), Some("Health check timed out"));
    let healthy = filter_healthy_agents(agents, &SlowTransport, &settings(5.0, 0.05)).await;
    assert!(healthy.is_empty());
}

#[tokio::test]
async fn an_invalid_probe_timeout_marks_probed_agents_unhealthy() {
    let transport = FakeTransport::answering(200);
    let agents = vec![agent("a1", Some("https://agent.example/one"))];
    let results = probe_agents(&agents, &transport, &settings(-1.0, 30.0)).await;
    assert_eq!(results.len(), 1);
    assert!(!results[0].healthy);
    assert_eq!(
        results[0].error.as_deref(),
        Some("the health check timeout is not a valid duration")
    );
    assert!(transport.probed().is_empty());
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}

fn response(status: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut text = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        text.push_str(name);
        text.push_str(": ");
        text.push_str(value);
        text.push_str("\r\n");
    }
    text.push_str("content-length: 0\r\nconnection: close\r\n\r\n");
    text.into_bytes()
}

struct ScriptedServer {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

async fn scripted_server(responses: Vec<Vec<u8>>) -> ScriptedServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        for answer in responses {
            let Ok((mut stream, _peer)) = listener.accept().await else {
                break;
            };
            let mut buffer = [0_u8; 2048];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8(buffer[..read].to_vec()).unwrap_or_default();
            seen.lock().unwrap().push(request);
            if stream.write_all(&answer).await.is_err() {
                break;
            }
        }
    });
    ScriptedServer {
        base: format!("http://{address}"),
        requests,
        _tasks: tasks,
    }
}

#[tokio::test]
async fn production_transport_follows_redirects_with_a_plain_get() {
    let server = scripted_server(vec![
        response("302 Found", &[("location", "/healthy")]),
        response("200 OK", &[]),
    ])
    .await;
    let transport = PooledHealthTransport {
        client: proxied_client(),
    };
    let status = transport
        .get_status(&format!("{}/start", server.base), FIVE_SECONDS)
        .await
        .unwrap();
    assert_eq!(status, 200);
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /start "), "{}", requests[0]);
    assert!(requests[1].starts_with("GET /healthy "), "{}", requests[1]);
    assert!(!requests[0].to_ascii_lowercase().contains("authorization"));
}

#[tokio::test]
async fn production_transport_reports_the_status_without_a_body() {
    let server = scripted_server(vec![response("503 Service Unavailable", &[])]).await;
    let transport = PooledHealthTransport {
        client: proxied_client(),
    };
    let status = transport
        .get_status(&format!("{}/status", server.base), FIVE_SECONDS)
        .await
        .unwrap();
    assert_eq!(status, 503);
}

#[tokio::test]
async fn production_transport_rejects_an_invalid_url() {
    let transport = PooledHealthTransport {
        client: proxied_client(),
    };
    let error = transport
        .get_status("not a url", FIVE_SECONDS)
        .await
        .unwrap_err();
    assert!(matches!(error, AgentHealthError::Url(_)));
}
