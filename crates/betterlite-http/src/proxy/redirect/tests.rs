#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderMap, HeaderValue};
use reqwest::{Body, Method, Url};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::proxy::limits::TransportLimits;
use crate::proxy::outbound::{OutboundRequest, RequestBody};
use crate::proxy::transport_error::TransportError;
use crate::proxy::{ProxiedClient, base_builder};
use crate::user_url::httpx_url::HttpxUrl;

use super::{MAX_REDIRECTS, next_target, send};

fn reply(status: u16, headers: &[String], body: &str) -> Vec<u8> {
    let mut head = format!("HTTP/1.1 {status} X\r\n");
    for header in headers {
        head.push_str(header);
        head.push_str("\r\n");
    }
    head.push_str(&format!(
        "content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    ));
    head.into_bytes()
}

fn moved(status: u16, location: &str) -> Vec<u8> {
    reply(status, &[format!("location: {location}")], "")
}

fn done(body: &str) -> Vec<u8> {
    reply(200, &[], body)
}

struct Scripted {
    base: Url,
    heads: Arc<Mutex<Vec<String>>>,
    _tasks: JoinSet<()>,
}

async fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        received.extend_from_slice(chunk.get(..read)?);
    }
    let head = String::from_utf8(received).ok()?;
    let length: usize = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length: ")?.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.ok()?;
    Some(format!("{head}{}", String::from_utf8(body).ok()?))
}

async fn accept_loop(
    listener: TcpListener,
    heads: Arc<Mutex<Vec<String>>>,
    responses: Arc<Mutex<VecDeque<Vec<u8>>>>,
) {
    while let Ok((mut stream, _)) = listener.accept().await {
        let Some(request) = read_request(&mut stream).await else {
            continue;
        };
        heads.lock().unwrap().push(request);
        let response = responses.lock().unwrap().pop_front();
        if let Some(response) = response {
            let _written = stream.write_all(&response).await;
        }
    }
}

async fn scripted(responses: Vec<Vec<u8>>) -> Scripted {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let heads: Arc<Mutex<Vec<String>>> = Arc::default();
    let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
    let mut tasks = JoinSet::new();
    tasks.spawn(accept_loop(listener, Arc::clone(&heads), responses));
    Scripted {
        base,
        heads,
        _tasks: tasks,
    }
}

fn client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}

fn post(url: Url) -> OutboundRequest {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer key"));
    headers.insert(COOKIE, HeaderValue::from_static("sid=1"));
    OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(b"{\"a\":1}".to_vec()),
        deadline: Duration::from_secs(5),
    }
}

fn header(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (found, value) = line.split_once(": ")?;
        (found.to_ascii_lowercase() == name).then(|| value.to_owned())
    })
}

fn body(head: &str) -> &str {
    head.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

fn line(head: &str) -> &str {
    head.lines().next().unwrap_or_default()
}

fn described(head: &str) -> String {
    let authorization = header(head, "authorization").unwrap_or_else(|| "-".to_owned());
    let cookie = header(head, "cookie").unwrap_or_else(|| "-".to_owned());
    let content_length = header(head, "content-length").unwrap_or_else(|| "-".to_owned());
    format!(
        "{}|auth={authorization}|cookie={cookie}|length={content_length}|body={}",
        line(head),
        body(head)
    )
}

async fn chain(status: u16) -> String {
    let server = scripted(vec![moved(status, "/next"), done("ok")]).await;
    let _answer = client()
        .send_buffered(post(server.base.join("start").unwrap()))
        .await;
    let heads = server.heads.lock().unwrap().clone();
    heads
        .iter()
        .map(|head| described(head))
        .collect::<Vec<_>>()
        .join(" ")
}

async fn second_hop(location: &str, path: &str) -> String {
    let server = scripted(vec![moved(302, location), done("ok")]).await;
    let _answer = client()
        .send_buffered(post(server.base.join(path).unwrap()))
        .await;
    let heads = server.heads.lock().unwrap().clone();
    heads
        .get(1)
        .map_or_else(String::new, |head| described(head))
}

async fn hop_bound() -> String {
    let server = scripted(vec![moved(302, "/loop"); MAX_REDIRECTS + 2]).await;
    let error = client()
        .send_buffered(post(server.base.clone()))
        .await
        .expect_err("the hop bound must end the chain");
    format!("{error} requests={}", server.heads.lock().unwrap().len())
}

async fn cross_origin() -> String {
    let target = scripted(vec![done("ok")]).await;
    let location = format!("{}next", target.base);
    let source = scripted(vec![moved(302, &location)]).await;
    let mut request = post(source.base.clone());
    request
        .headers
        .insert("x-api-key", HeaderValue::from_static("k1"));
    request
        .headers
        .insert("x-goog-api-key", HeaderValue::from_static("k2"));
    request
        .headers
        .insert("x-provider-credential", HeaderValue::from_static("secret"));
    request
        .headers
        .get_mut("x-provider-credential")
        .unwrap()
        .set_sensitive(true);
    request
        .headers
        .insert("x-provider-trace", HeaderValue::from_static("trace"));
    let _answer = client().send_buffered(request).await;
    let heads = target.heads.lock().unwrap().clone();
    let Some(head) = heads.first() else {
        return "nothing reached the target".to_owned();
    };
    format!(
        "{} auth={}|cookie={}|apikey={}|goog={}|credential={}|trace={}",
        line(head),
        header(head, "authorization").is_some(),
        header(head, "cookie").is_some(),
        header(head, "x-api-key").is_some(),
        header(head, "x-goog-api-key").is_some(),
        header(head, "x-provider-credential").is_some(),
        header(head, "x-provider-trace").is_some(),
    )
}

async fn streamed_answer() -> String {
    let server = scripted(vec![
        reply(302, &[String::from("location: /next")], "redirect-body"),
        reply(200, &[], "data: 1"),
    ])
    .await;
    let request = OutboundRequest {
        method: Method::GET,
        url: server.base.clone(),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_secs(5),
    };
    let answer = client().send_buffered(request).await.unwrap();
    String::from_utf8(answer.body).unwrap()
}

async fn unrepeatable_body() -> String {
    let server = scripted(vec![moved(307, "/next")]).await;
    let stream = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(b"chunk".to_vec())]);
    let request = client()
        .client
        .post(server.base.clone())
        .body(Body::wrap_stream(stream))
        .build()
        .unwrap();
    match send(&client(), request).await {
        Ok(_answer) => "followed".to_owned(),
        Err(error) => error.to_string(),
    }
}

fn refusal(location: &str) -> String {
    let current = HttpxUrl::parse("https://api.example.com/v1/jobs").unwrap();
    match next_target(&current, location) {
        Ok(target) => format!("followed to {}", target.href()),
        Err(error) => error.to_string(),
    }
}

async fn observe(case: Case) -> String {
    match case {
        Case::Post301 => chain(301).await,
        Case::Post302 => chain(302).await,
        Case::Post303 => chain(303).await,
        Case::Post307 => chain(307).await,
        Case::Post308 => chain(308).await,
        Case::Relative => second_hop("../next?x=1", "a/b/start").await,
        Case::HopBound => hop_bound().await,
        Case::CrossOrigin => cross_origin().await,
        Case::SameOrigin => second_hop("/next", "").await,
        Case::StreamedAnswer => streamed_answer().await,
        Case::UnrepeatableBody => unrepeatable_body().await,
        Case::HttpsDowngrade => refusal("http://api.example.com/v1/jobs/1"),
        Case::RedirectUserinfo => refusal("https://user:pass@api.example.com/v1/jobs/1"),
    }
}

#[derive(Clone, Copy)]
enum Case {
    Post301,
    Post302,
    Post303,
    Post307,
    Post308,
    Relative,
    HopBound,
    CrossOrigin,
    SameOrigin,
    StreamedAnswer,
    UnrepeatableBody,
    HttpsDowngrade,
    RedirectUserinfo,
}

const GONE: &str = "POST /start|auth=Bearer key|cookie=sid=1|length=7|body={\"a\":1}";

#[rstest]
#[case::a_301_turns_a_post_into_a_get(Case::Post301, &format!("{GONE} GET /next|auth=Bearer key|cookie=sid=1|length=-|body="))]
#[case::a_302_turns_a_post_into_a_get(Case::Post302, &format!("{GONE} GET /next|auth=Bearer key|cookie=sid=1|length=-|body="))]
#[case::a_303_turns_a_post_into_a_get(Case::Post303, &format!("{GONE} GET /next|auth=Bearer key|cookie=sid=1|length=-|body="))]
#[case::a_307_replays_the_post_body(Case::Post307, &format!("{GONE} POST /next|auth=Bearer key|cookie=sid=1|length=7|body={{\"a\":1}}"))]
#[case::a_308_replays_the_post_body(Case::Post308, &format!("{GONE} POST /next|auth=Bearer key|cookie=sid=1|length=7|body={{\"a\":1}}"))]
#[case::relative_location_joins_against_the_request(
    Case::Relative,
    "GET /a/next?x=1 HTTP/1.1|auth=Bearer key|cookie=sid=1|length=-|body="
)]
#[case::the_hop_bound_stops_the_chain(
    Case::HopBound,
    "redirected the request more than 10 times requests=11"
)]
#[case::credentials_are_stripped_across_origins(
    Case::CrossOrigin,
    "GET /next HTTP/1.1 auth=false|cookie=false|apikey=false|goog=false|credential=false|trace=true"
)]
#[case::same_origin_keeps_credentials(
    Case::SameOrigin,
    "GET /next|auth=Bearer key|cookie=sid=1|length=-|body="
)]
#[case::a_redirect_body_never_reaches_the_stream(Case::StreamedAnswer, "data: 1")]
#[case::a_307_cannot_replay_a_streaming_body(
    Case::UnrepeatableBody,
    "the redirect cannot replay a request body that streams"
)]
#[case::an_https_downgrade_is_refused(
    Case::HttpsDowngrade,
    "the provider redirected an HTTPS request to HTTP"
)]
#[case::a_redirect_with_userinfo_is_refused(
    Case::RedirectUserinfo,
    "the redirect target carries a user name or password"
)]
#[tokio::test]
async fn http_boundaries_url_boundary_tests_behavior(#[case] case: Case, #[case] expected: &str) {
    assert_eq!(observe(case).await, expected);
}
