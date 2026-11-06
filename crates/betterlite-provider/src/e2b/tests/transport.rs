use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::Environment;
use crate::e2b::transport::ProxiedSandboxHttp;
use crate::e2b::{CreateSandbox, DeleteSandbox, E2bError, E2bSandbox, SandboxRef};

async fn answer_once(listener: TcpListener, status: &str, body: &str) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let head_text = String::from_utf8(head).unwrap();
    let length: usize = head_text
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    stream.read_exact(&mut sent).await.unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    format!("{head_text}{}", String::from_utf8(sent).unwrap())
}

fn pooled_http() -> ProxiedSandboxHttp {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedSandboxHttp {
        client: ProxiedClient::outbound(&policy).unwrap(),
    }
}

#[tokio::test]
async fn create_goes_through_the_pooled_http_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server = JoinSet::new();
    server.spawn(answer_once(
        listener,
        "200 OK",
        r#"{"sandboxID":"sbx_7","envdAccessToken":"envd"}"#,
    ));
    let sandbox = E2bSandbox {
        http: pooled_http(),
        environment: Environment::new(),
    };
    let api_base = format!("http://{address}");
    let created = sandbox
        .acreate_sandbox(CreateSandbox {
            template: None,
            timeout: None,
            allow_internet_access: None,
            api_key: Some("e2b-key"),
            api_base: Some(&api_base),
        })
        .await
        .unwrap();
    assert_eq!(created.id, "sbx_7");
    let seen = server.join_next().await.unwrap().unwrap();
    assert!(seen.starts_with("POST /sandboxes HTTP/1.1"));
    assert!(seen.contains(r#""templateID":"code-interpreter-v1""#));
    assert!(seen.to_ascii_lowercase().contains("x-api-key: e2b-key"));
}

#[tokio::test]
async fn delete_returns_false_through_the_pooled_http_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server = JoinSet::new();
    server.spawn(answer_once(listener, "404 Not Found", ""));
    let sandbox = E2bSandbox {
        http: pooled_http(),
        environment: Environment::new(),
    };
    let api_base = format!("http://{address}");
    let deleted = sandbox
        .adelete_sandbox(DeleteSandbox {
            container: SandboxRef::Id("sbx_9"),
            api_key: Some("e2b-key"),
            api_base: Some(&api_base),
        })
        .await
        .unwrap();
    assert!(!deleted);
    let seen = server.join_next().await.unwrap().unwrap();
    assert!(seen.starts_with("DELETE /sandboxes/sbx_9 HTTP/1.1"));
}

#[tokio::test]
async fn create_reports_a_server_error_through_the_pooled_http_client() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut server = JoinSet::new();
    server.spawn(answer_once(listener, "500 Internal Server Error", "{}"));
    let sandbox = E2bSandbox {
        http: pooled_http(),
        environment: Environment::new(),
    };
    let api_base = format!("http://{address}");
    assert!(matches!(
        sandbox
            .acreate_sandbox(CreateSandbox {
                template: None,
                timeout: None,
                allow_internet_access: None,
                api_key: Some("e2b-key"),
                api_base: Some(&api_base),
            })
            .await
            .unwrap_err(),
        E2bError::HttpStatus { status: 500 }
    ));
    server.join_next().await.unwrap().unwrap();
}
