use std::str::from_utf8;

use tokio::time::timeout;

use super::*;

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn form() -> Vec<(String, SecretString)> {
    vec![
        ("grant_type".to_owned(), secret("authorization_code")),
        ("client_id".to_owned(), secret("client-1")),
        ("client_secret".to_owned(), secret("s e/c")),
    ]
}

fn poster(limits: TransportLimits) -> HttpTokenPost {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    HttpTokenPost {
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
    let Ok(text) = from_utf8(request) else {
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

#[tokio::test]
async fn posts_the_form_urlencoded_body_to_the_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(answer(
        listener,
        response("HTTP/1.1 200 OK", r#"{"access_token":"minted"}"#),
    ));
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form())
        .await;
    let request = String::from_utf8(tasks.join_next().await.unwrap().unwrap()).unwrap();
    assert!(request.starts_with("POST /token HTTP/1.1\r\n"));
    assert!(request.contains("content-type: application/x-www-form-urlencoded\r\n"));
    assert!(
        request.ends_with("grant_type=authorization_code&client_id=client-1&client_secret=s+e%2Fc")
    );
    let PostedForm::Answered { status, body } = outcome else {
        panic!("the endpoint answered");
    };
    assert_eq!(status, 200);
    assert_eq!(from_utf8(&body).unwrap(), r#"{"access_token":"minted"}"#);
}

#[tokio::test]
async fn reports_a_connection_failure_as_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form())
        .await;
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the endpoint is closed");
    };
    assert_eq!(kind, "TransportError");
}

#[tokio::test]
async fn reports_a_body_past_the_limit_as_unreachable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(answer(
        listener,
        response("HTTP/1.1 200 OK", &"x".repeat(64)),
    ));
    let limits = TransportLimits {
        connect_timeout: Duration::from_secs(5),
        read_timeout: Duration::from_secs(5),
        body_bytes: 16,
        header_bytes: 4096,
    };
    let outcome = poster(limits)
        .post(&format!("http://{address}/token"), &form())
        .await;
    drop(tasks.join_next().await);
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the body is past the limit");
    };
    assert_eq!(kind, "TransportError");
}

#[tokio::test]
async fn reports_a_url_that_cannot_be_parsed() {
    let outcome = poster(TransportLimits::DEFAULT)
        .post("not a url", &form())
        .await;
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the url cannot be parsed");
    };
    assert_eq!(kind, "InvalidUrl");
}

#[tokio::test]
async fn a_dropped_connection_is_attempted_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut stream, _peer) = listener.accept().await.expect("the client connects");
        drop(read_request(&mut stream).await);
        drop(stream);
        let retried = timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(retried.is_err(), "the failure is not retried");
    });
    let outcome = poster(TransportLimits::DEFAULT)
        .post(&format!("http://{address}/token"), &form())
        .await;
    let PostedForm::Unreachable { kind } = outcome else {
        panic!("the endpoint dropped the connection");
    };
    assert_eq!(kind, "TransportError");
    tasks.join_next().await.unwrap().unwrap();
}
