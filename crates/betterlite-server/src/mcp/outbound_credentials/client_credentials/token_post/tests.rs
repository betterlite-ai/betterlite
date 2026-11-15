#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::{HttpGrantPost, M2mTokenEndpointPost as _, TokenEndpointOutcome, parse_token_response};

const NON_OBJECT_DETAIL: &str = "token endpoint returned a non-JSON-object body";

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn form() -> Vec<(String, SecretString)> {
    vec![
        ("grant_type".to_owned(), secret("client_credentials")),
        ("client_id".to_owned(), secret("client")),
        ("client_secret".to_owned(), secret("s e/c")),
    ]
}

fn poster(limits: TransportLimits) -> HttpGrantPost {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    HttpGrantPost {
        client: Arc::new(ProxiedClient::build(base_builder(), &policy, limits).unwrap()),
        deadline: Duration::from_secs(5),
    }
}

fn response(status_line: &str, body: &str) -> String {
    format!(
        "{status_line}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut collected = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !complete(&collected) {
        let read = stream.read(&mut buffer).await.expect("the request arrives");
        if read == 0 {
            break;
        }
        collected.extend_from_slice(&buffer[..read]);
    }
    collected
}

fn complete(request: &[u8]) -> bool {
    let Ok(text) = str::from_utf8(request) else {
        return false;
    };
    let Some(split) = text.find("\r\n\r\n") else {
        return false;
    };
    let Some(head) = text.get(..split) else {
        return false;
    };
    let declared = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    declared.is_none_or(|length| {
        split
            .checked_add(4)
            .and_then(|start| start.checked_add(length))
            .is_some_and(|total| text.len() >= total)
    })
}

async fn answer(listener: TcpListener, reply: String) -> Vec<u8> {
    let (mut stream, _peer) = listener.accept().await.expect("the client connects");
    let request = read_request(&mut stream).await;
    stream
        .write_all(reply.as_bytes())
        .await
        .expect("the response is written");
    stream.flush().await.expect("the response is flushed");
    request
}

async fn exchange(reply: String, post: HttpGrantPost) -> (String, TokenEndpointOutcome) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(answer(listener, reply));
    let outcome = post
        .post(&format!("http://{address}/token"), &form(), &[])
        .await;
    let request = tasks.join_next().await.unwrap().unwrap();
    (String::from_utf8(request).unwrap(), outcome)
}

#[tokio::test]
async fn sends_the_form_urlencoded_with_an_accept_header() {
    let reply = response("HTTP/1.1 200 OK", "{\"access_token\":\"tok\"}");
    let (request, outcome) = exchange(reply, poster(TransportLimits::DEFAULT)).await;
    assert!(request.starts_with("POST /token HTTP/1.1\r\n"));
    assert!(request.contains("accept: application/json\r\n"));
    assert!(request.contains("content-type: application/x-www-form-urlencoded\r\n"));
    assert!(
        request.ends_with("grant_type=client_credentials&client_id=client&client_secret=s+e%2Fc")
    );
    let TokenEndpointOutcome::Success { body } = outcome else {
        panic!("a 200 with a JSON object is a success");
    };
    assert_eq!(body.access_token_text(), Some("tok"));
}

#[tokio::test]
async fn a_grant_header_wins_over_the_accept_header() {
    let reply = response("HTTP/1.1 200 OK", "{}");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(answer(listener, reply));
    let headers = vec![("Accept".to_owned(), secret("text/plain"))];
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form(), &headers)
        .await;
    let request = String::from_utf8(tasks.join_next().await.unwrap().unwrap()).unwrap();
    assert!(request.contains("accept: text/plain\r\n"));
    assert!(!request.contains("accept: application/json"));
    assert!(matches!(outcome, TokenEndpointOutcome::Success { .. }));
}

#[rstest]
#[case::bad_request(400, "HTTP/1.1 400 Bad Request")]
#[case::server_error(500, "HTTP/1.1 500 Internal Server Error")]
#[tokio::test]
async fn a_non_2xx_status_is_denied_with_its_status_code(
    #[case] status: u16,
    #[case] status_line: &str,
) {
    let reply = response(status_line, "occluded!");
    let (_request, outcome) = exchange(reply, poster(TransportLimits::DEFAULT)).await;
    let TokenEndpointOutcome::Denied {
        status_code,
        detail,
    } = outcome
    else {
        panic!("a non-2xx status is denied");
    };
    assert_eq!(status_code, status);
    assert_eq!(detail, format!("token endpoint returned HTTP {status}"));
}

#[tokio::test]
async fn a_refused_connection_is_unreachable_without_the_url() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form(), &[])
        .await;
    let TokenEndpointOutcome::Unreachable { detail } = outcome else {
        panic!("a refused connection is unreachable");
    };
    assert_eq!(detail, "Connect");
    assert!(!detail.contains(&address.to_string()));
}

#[tokio::test]
async fn a_body_over_the_pool_cap_is_unreachable() {
    let reply = response(
        "HTTP/1.1 200 OK",
        "{\"access_token\":\"abcdefghijklmnopqrstuvwxyz\"}",
    );
    let limits = TransportLimits {
        body_bytes: 16,
        ..TransportLimits::DEFAULT
    };
    let (_request, outcome) = exchange(reply, poster(limits)).await;
    let TokenEndpointOutcome::Unreachable { detail } = outcome else {
        panic!("a body over the cap is unreachable");
    };
    assert_eq!(detail, "BodyTooLarge");
}

#[tokio::test]
async fn a_url_that_does_not_parse_is_unreachable() {
    let outcome = poster(TransportLimits::DEFAULT)
        .post("not a url", &form(), &[])
        .await;
    let TokenEndpointOutcome::Unreachable { detail } = outcome else {
        panic!("an unparsable token URL cannot be reached");
    };
    assert_eq!(detail, "InvalidUrl");
}

#[rstest]
#[case::object("{\"access_token\":\"tok\"}", true)]
#[case::array("[1,2,3]", false)]
#[case::string_array("[\"abc\",3600,\"read\"]", false)]
#[case::string("\"tok\"", false)]
#[case::null("null", false)]
#[case::number("7", false)]
#[case::empty("", false)]
#[case::words("not json", false)]
#[case::nested("{\"access_token\":{\"value\":\"tok\"}}", true)]
#[case::float_expiry("{\"access_token\":\"tok\",\"expires_in\":3600.5}", true)]
fn parse_token_response_accepts_only_a_json_object(#[case] raw: &str, #[case] object: bool) {
    match parse_token_response(raw.as_bytes(), 204) {
        TokenEndpointOutcome::Success { .. } => assert!(object),
        TokenEndpointOutcome::Denied {
            status_code,
            detail,
        } => {
            assert!(!object);
            assert_eq!(status_code, 204);
            assert_eq!(detail, NON_OBJECT_DETAIL);
        }
        TokenEndpointOutcome::Unreachable { .. } => {
            panic!("a decode never reports an unreachable endpoint");
        }
    }
}

#[test]
fn parse_token_response_reads_only_a_string_access_token() {
    let outcome = parse_token_response(b"{\"access_token\": 12, \"expires_in\": true}", 200);
    let TokenEndpointOutcome::Success { body } = outcome else {
        panic!("an object with a non-string token is still a success");
    };
    assert_eq!(body.access_token_text(), None);
}

#[rstest]
fn replays_the_token_response_corpus(#[files("fuzz/corpus/token_response/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus seed is readable");
    match parse_token_response(&bytes, 200) {
        TokenEndpointOutcome::Success { .. } => {}
        TokenEndpointOutcome::Denied {
            status_code,
            detail,
        } => {
            assert_eq!(status_code, 200);
            assert_eq!(detail, NON_OBJECT_DETAIL);
        }
        TokenEndpointOutcome::Unreachable { .. } => {
            panic!("a decode never reports an unreachable endpoint");
        }
    }
}
