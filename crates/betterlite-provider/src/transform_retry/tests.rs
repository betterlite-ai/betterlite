#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::controls::json_value::JsonValue;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

use super::{TransformOnUnprocessable, TransformRetry, send_with_retry};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest, Upstream};

mod bounds;

struct Rewrite;

impl TransformOnUnprocessable for Rewrite {
    fn should_retry(&self, response: &UpstreamResponse, _deployment: &Deployment) -> bool {
        response.status == 422
    }

    fn transform_request(
        &self,
        prepared: &mut PreparedRequest,
        _response: &UpstreamResponse,
        _deployment: &Deployment,
    ) {
        if let PreparedRequest::Chat(chat) = prepared {
            chat.http.body = b"rewritten".to_vec();
        }
    }
}

async fn serve(listener: TcpListener, statuses: Vec<u16>) -> Vec<String> {
    let mut bodies = Vec::new();
    for status in statuses {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(socket.read_u8().await.unwrap());
        }
        let length: usize = String::from_utf8(head)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        let mut body = vec![0_u8; length];
        socket.read_exact(&mut body).await.unwrap();
        bodies.push(String::from_utf8(body).unwrap());
        let reply =
            format!("HTTP/1.1 {status} X\r\ncontent-length: 2\r\nconnection: close\r\n\r\n");
        socket.write_all(reply.as_bytes()).await.unwrap();
        socket.write_all(b"[]").await.unwrap();
    }
    bodies
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn prepared(url: Url, retry: TransformRetry) -> PreparedRequest {
    PreparedRequest::Chat(Box::new(PreparedChat {
        http: PreparedHttp {
            method: Method::POST,
            url,
            headers: HeaderMap::new(),
            body: b"original".to_vec(),
            deadline: Duration::from_secs(5),
            body_limit: None,
        },
        retry,
        client_model: None,
    }))
}

fn deployment() -> Deployment {
    let params = [(String::from("model"), JsonValue::Text(String::from("up")))];
    Deployment::from_params(IndexMap::from(params)).unwrap()
}

#[rstest]
#[case::disabled_makes_one_attempt(TransformRetry::DISABLED, vec![422], 422, vec!["original"])]
#[case::retries_until_success(TransformRetry::bounded(3).unwrap(), vec![422, 422, 200], 200, vec!["original", "rewritten", "rewritten"])]
#[case::stops_at_the_bound(TransformRetry::bounded(2).unwrap(), vec![422, 422], 422, vec!["original", "rewritten"])]
#[case::hook_declines_other_statuses(TransformRetry::bounded(3).unwrap(), vec![500], 500, vec!["original"])]
#[tokio::test]
async fn provider_framework_tests_behavior(
    #[case] retry: TransformRetry,
    #[case] statuses: Vec<u16>,
    #[case] last: u16,
    #[case] sent: Vec<&str>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let client = client();
    let request = prepared(url, retry);
    let inputs = deployment();
    let (bodies, answer) = tokio::join!(
        serve(listener, statuses),
        send_with_retry(&client, &request, &inputs, &Rewrite)
    );
    let Upstream::Buffered(response) = answer.unwrap() else {
        panic!("a plain buffered chat answers buffered");
    };
    assert_eq!(response.status, last);
    assert_eq!(bodies, sent);
}
