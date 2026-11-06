#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_roi::settings::ROISettings;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::client::GitHub;
use super::support::{Script, github, json_reply, reply};

#[tokio::test]
async fn a_scripted_redirect_is_returned_instead_of_followed() {
    let mut redirect = reply(302);
    redirect.headers.push((
        "location".to_owned(),
        "https://elsewhere.example/next".to_owned(),
    ));
    let script = Script::new(vec![redirect, json_reply(200, "[]")]);
    let github = github(script.clone());
    let response = github
        .request(Method::GET, "user/repos", &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(
        response.headers.get("location").unwrap(),
        "https://elsewhere.example/next"
    );
    assert_eq!(script.seen().len(), 1);
}

#[tokio::test]
async fn the_pooled_transport_does_not_follow_a_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0_u8; 4096];
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        socket
            .write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        timeout(Duration::from_millis(300), listener.accept())
            .await
            .is_err()
    });
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let settings: ROISettings =
        serde_json::from_str(&format!("{{\"github_api_url\":\"http://{address}\"}}")).unwrap();
    let github = GitHub::new(&settings, client).unwrap();
    let response = github
        .request(Method::GET, "repos/octo/repo", &[], None)
        .await
        .unwrap();
    assert_eq!(response.status, 302);
    assert!(tasks.join_next().await.unwrap().unwrap());
}
