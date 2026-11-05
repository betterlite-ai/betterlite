#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use rstest::rstest;
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::{ClientChoice, ClientPool, PoolError};
use crate::proxy::ProxiedClient;
use crate::proxy::limits::TransportLimits;
use crate::proxy::outbound::{OutboundRequest, RequestBody};
use crate::proxy::tests::{default_client, kind};

const SERVER_CERT: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.pem");
const SERVER_KEY: &[u8] =
    include_bytes!("../../../../../tools/postgres-fixture/fixtures/tls/server.key");

fn tls(verification: Verification) -> TlsPolicy {
    TlsPolicy {
        verification,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite".to_owned(),
    }
}

fn outcome(selected: Result<ClientChoice, PoolError>) -> String {
    selected.map_or_else(|error| kind(&error), |choice| kind(&choice))
}

#[derive(Clone, Copy)]
enum Scenario {
    SuppliedMockBroken,
    MockBroken,
    Enabled,
    EnabledTwice,
    EnabledThenDisabled,
    Broken,
    DrainedThenDisabled,
    DeploymentTls,
}

async fn status(client: &ProxiedClient, url: &Url) -> String {
    let request = OutboundRequest {
        method: Method::GET,
        url: url.clone(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(5),
    };
    match client.send_buffered(request).await {
        Ok(response) => response.status.to_string(),
        Err(error) => kind(&error),
    }
}

async fn serve_tls(listener: &TcpListener, acceptor: &TlsAcceptor) {
    let (socket, _) = listener.accept().await.unwrap();
    if let Ok(mut stream) = acceptor.accept(socket).await {
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.unwrap());
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    }
}

async fn deployment_tls(
    pool: &ClientPool,
    routing: &Arc<ProxyPolicy>,
    default: &TlsPolicy,
    lax: &TlsPolicy,
) -> String {
    let mut base = pool.client(default, routing).unwrap();
    let sibling = pool.client(lax, routing).unwrap();
    base.deployments = Arc::new(BTreeMap::from([("lax".to_owned(), sibling)]));
    let certificates = CertificateDer::pem_slice_iter(SERVER_CERT)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let origin = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            certificates,
            PrivateKeyDer::from_pem_slice(SERVER_KEY).unwrap(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(origin));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: Url = format!("https://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let served = async {
        serve_tls(&listener, &acceptor).await;
        serve_tls(&listener, &acceptor).await;
    };
    let requests = async {
        let strict = status(base.deployments.get("other").unwrap_or(&base), &url).await;
        let relaxed = status(base.deployments.get("lax").unwrap_or(&base), &url).await;
        format!("{strict} {relaxed}")
    };
    tokio::join!(served, requests).1
}

#[rstest]
#[case::mock_after_supplied_client_async(1, Scenario::SuppliedMockBroken, "Supplied clients=0")]
#[case::mock_after_supplied_client_sync(1, Scenario::SuppliedMockBroken, "Supplied clients=0")]
#[case::mock_before_network_options_async(1, Scenario::MockBroken, "Mock clients=0")]
#[case::mock_before_network_options_sync(1, Scenario::MockBroken, "Mock clients=0")]
#[case::unsupplied_unmocked_selection_builds_a_pooled_client(
    1,
    Scenario::Enabled,
    "Pooled clients=1"
)]
#[case::the_same_policy_reuses_one_client(2, Scenario::EnabledTwice, "Pooled Pooled clients=1")]
#[case::different_tls_policies_stay_isolated(
    2,
    Scenario::EnabledThenDisabled,
    "Pooled Pooled clients=2"
)]
#[case::the_pool_is_bound_to_the_configured_deployments(
    1,
    Scenario::EnabledThenDisabled,
    "Pooled Full clients=1"
)]
#[case::an_unreadable_ca_file_is_an_error_not_a_client(1, Scenario::Broken, "Client clients=0")]
#[case::a_drained_pool_takes_no_new_policy(
    2,
    Scenario::DrainedThenDisabled,
    "Pooled drained Closed clients=1"
)]
#[case::a_deployment_ssl_verify_selects_its_own_client(
    2,
    Scenario::DeploymentTls,
    "Connect 200 clients=2"
)]
#[tokio::test]
async fn http_pool_tests_behavior(
    #[case] capacity: usize,
    #[case] scenario: Scenario,
    #[case] expected: &str,
) {
    let pool = ClientPool::new(capacity, TransportLimits::DEFAULT);
    let routing = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let broken = TlsPolicy {
        ca_file: Some("/nonexistent/betterlite-ca.pem".to_owned()),
        ..tls(Verification::Enabled)
    };
    let enabled = tls(Verification::Enabled);
    let disabled = tls(Verification::Disabled);
    let observed = match scenario {
        Scenario::SuppliedMockBroken => {
            outcome(pool.select(Some(default_client()), true, &broken, &routing))
        }
        Scenario::MockBroken => outcome(pool.select(None, true, &broken, &routing)),
        Scenario::Enabled => outcome(pool.select(None, false, &enabled, &routing)),
        Scenario::EnabledTwice => {
            let first = outcome(pool.select(None, false, &enabled, &routing));
            format!(
                "{first} {}",
                outcome(pool.select(None, false, &enabled, &routing))
            )
        }
        Scenario::EnabledThenDisabled => {
            let first = outcome(pool.select(None, false, &enabled, &routing));
            format!(
                "{first} {}",
                outcome(pool.select(None, false, &disabled, &routing))
            )
        }
        Scenario::Broken => outcome(pool.select(None, false, &broken, &routing)),
        Scenario::DrainedThenDisabled => {
            let first = outcome(pool.select(None, false, &enabled, &routing));
            let client = pool.client(&enabled, &routing).unwrap();
            client.drain(Duration::from_secs(1)).await.unwrap();
            let refused = outcome(pool.select(None, false, &disabled, &routing));
            format!("{first} drained {refused}")
        }
        Scenario::DeploymentTls => deployment_tls(&pool, &routing, &enabled, &disabled).await,
    };
    assert_eq!(
        format!("{observed} clients={}", pool.client_count()),
        expected
    );
}
