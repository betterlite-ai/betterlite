use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::routing::any;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use http::Method;
use http::header::{HeaderMap, HeaderName};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use url::Url;

struct Captured {
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Seen(Arc<Mutex<Vec<Captured>>>);

async fn capture(State(seen): State<Seen>, request: Request) -> StatusCode {
    let method = request.method().clone();
    let uri = request.uri().to_string();
    let headers = request.headers().clone();
    let body = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    seen.0.lock().unwrap().push(Captured {
        method,
        uri,
        headers,
        body,
    });
    StatusCode::OK
}

async fn upstream() -> (Url, Seen, JoinSet<()>) {
    let seen = Seen(Arc::new(Mutex::new(Vec::new())));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let application = Router::new()
        .route("/capture", any(capture))
        .with_state(seen.clone());
    let mut server = JoinSet::new();
    server.spawn(async move { axum::serve(listener, application).await.unwrap() });
    (
        Url::parse(&format!("http://{address}/capture")).unwrap(),
        seen,
        server,
    )
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn header_text(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(HeaderName::from_static(name))
        .map(|value| value.to_str().unwrap().to_owned())
}

mod passthrough_non_streaming_http_handler {
    use std::time::Duration;

    use betterlite_core::api::request_form::values::{FormPart, FormParts, PartValue, UploadFile};
    use betterlite_http::proxy::ProxiedClient;
    use http::Method;
    use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
    use serde_json::json;
    use url::Url;

    use super::{Seen, client, header_text, upstream};
    use crate::passthrough::non_streaming::{
        NonStreamingError, NonStreamingRequest, non_streaming_request_handler,
    };
    use crate::passthrough::upstream::UpstreamTarget;

    const REBUILT_BODY: &str = concat!(
        "--rebuilt\r\n",
        "Content-Disposition: form-data; name=\"meta\"\r\n",
        "\r\n",
        "v1\r\n",
        "--rebuilt\r\n",
        "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "DATA\r\n",
        "--rebuilt--\r\n",
    );

    fn target(method: Method, url: Url, headers: HeaderMap) -> UpstreamTarget {
        UpstreamTarget {
            method,
            url,
            headers,
            query: vec![("page".to_owned(), "2".to_owned())],
            deadline: Duration::from_secs(5),
        }
    }

    fn form_request(
        method: Method,
        url: Url,
        headers: HeaderMap,
        form: &FormParts,
        forward_multipart: bool,
    ) -> NonStreamingRequest<'_> {
        NonStreamingRequest {
            target: target(method, url, headers),
            parsed_body: None,
            form: Some(form),
            forward_multipart,
            boundary: "rebuilt".to_owned(),
        }
    }

    fn json_request(url: Url, body: &serde_json::Value) -> NonStreamingRequest<'_> {
        NonStreamingRequest {
            target: target(Method::POST, url, HeaderMap::new()),
            parsed_body: Some(body),
            form: None,
            forward_multipart: false,
            boundary: String::new(),
        }
    }

    fn multipart_headers(content_type: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        headers
    }

    fn text(name: &str, value: &str) -> FormPart {
        FormPart {
            name: name.to_owned(),
            value: PartValue::Text(value.to_owned()),
        }
    }

    fn file(name: &str, filename: &str, content_type: Option<&str>, content: &[u8]) -> FormPart {
        FormPart {
            name: name.to_owned(),
            value: PartValue::Upload(UploadFile {
                filename: filename.to_owned(),
                content_type: content_type.map(str::to_owned),
                content: content.to_vec(),
                size: Some(u64::try_from(content.len()).unwrap()),
            }),
        }
    }

    async fn call(client: &ProxiedClient, request: NonStreamingRequest<'_>) -> u16 {
        non_streaming_request_handler(client, request)
            .await
            .unwrap()
            .buffered(4096)
            .await
            .unwrap()
            .status
    }

    fn captured_count(seen: &Seen) -> usize {
        seen.0.lock().unwrap().len()
    }

    #[tokio::test]
    async fn get_wins_over_a_multipart_form() {
        let (url, seen, _server) = upstream().await;
        let mut headers = multipart_headers("multipart/form-data; boundary=caller");
        headers.insert(
            HeaderName::from_static("x-forwarded"),
            HeaderValue::from_static("yes"),
        );
        let form = FormParts {
            items: vec![text("a", "1")],
        };
        let status = call(
            &client(),
            form_request(Method::GET, url, headers, &form, true),
        )
        .await;
        assert_eq!(status, 200);
        let captured = seen.0.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].method, Method::GET);
        assert_eq!(captured[0].uri, "/capture?page=2");
        assert!(captured[0].body.is_empty());
        assert_eq!(
            header_text(&captured[0].headers, "content-type").as_deref(),
            Some("multipart/form-data; boundary=caller")
        );
        assert_eq!(
            header_text(&captured[0].headers, "x-forwarded").as_deref(),
            Some("yes")
        );
    }

    #[tokio::test]
    async fn a_json_post_is_sent_as_json() {
        let (url, seen, _server) = upstream().await;
        let body = json!({"model": "demo"});
        let status = call(&client(), json_request(url, &body)).await;
        assert_eq!(status, 200);
        let captured = seen.0.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].method, Method::POST);
        assert_eq!(
            header_text(&captured[0].headers, "content-type").as_deref(),
            Some("application/json")
        );
        let expected: &[u8] = br#"{"model":"demo"}"#;
        assert_eq!(captured[0].body, expected);
    }

    #[tokio::test]
    async fn a_multipart_post_is_forwarded_with_the_rebuilt_boundary() {
        let (url, seen, _server) = upstream().await;
        let mut headers = multipart_headers("multipart/form-data; boundary=caller");
        headers.insert(
            HeaderName::from_static("x-custom"),
            HeaderValue::from_static("kept"),
        );
        let form = FormParts {
            items: vec![
                text("meta", "v1"),
                file("file", "a.txt", Some("text/plain"), b"DATA"),
            ],
        };
        let status = call(
            &client(),
            form_request(Method::POST, url, headers, &form, true),
        )
        .await;
        assert_eq!(status, 200);
        let captured = seen.0.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            header_text(&captured[0].headers, "content-type").as_deref(),
            Some("multipart/form-data; boundary=rebuilt")
        );
        assert_eq!(
            header_text(&captured[0].headers, "x-custom").as_deref(),
            Some("kept")
        );
        assert_eq!(captured[0].body, REBUILT_BODY.as_bytes());
    }

    #[tokio::test]
    async fn json_under_a_multipart_content_type_keeps_the_callers_content_type() {
        let (url, seen, _server) = upstream().await;
        let headers = multipart_headers("multipart/form-data; boundary=caller");
        let form = FormParts {
            items: vec![text("a", "1")],
        };
        let body = json!({"a": 1});
        let request = NonStreamingRequest {
            target: target(Method::POST, url, headers),
            parsed_body: Some(&body),
            form: Some(&form),
            forward_multipart: false,
            boundary: "rebuilt".to_owned(),
        };
        let status = call(&client(), request).await;
        assert_eq!(status, 200);
        let captured = seen.0.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(
            header_text(&captured[0].headers, "content-type").as_deref(),
            Some("multipart/form-data; boundary=caller")
        );
        let expected: &[u8] = br#"{"a":1}"#;
        assert_eq!(captured[0].body, expected);
    }

    #[tokio::test]
    async fn a_post_without_any_body_sends_nothing() {
        let (url, seen, _server) = upstream().await;
        let request = NonStreamingRequest {
            target: target(Method::POST, url, HeaderMap::new()),
            parsed_body: None,
            form: None,
            forward_multipart: false,
            boundary: String::new(),
        };
        let status = call(&client(), request).await;
        assert_eq!(status, 200);
        let captured = seen.0.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert!(captured[0].body.is_empty());
        assert_eq!(header_text(&captured[0].headers, "content-type"), None);
    }

    #[tokio::test]
    async fn a_multipart_request_without_its_form_is_rejected() {
        let (url, seen, _server) = upstream().await;
        let headers = multipart_headers("multipart/form-data");
        let request = NonStreamingRequest {
            target: target(Method::POST, url, headers),
            parsed_body: None,
            form: None,
            forward_multipart: true,
            boundary: "rebuilt".to_owned(),
        };
        match non_streaming_request_handler(&client(), request).await {
            Err(error) => assert!(matches!(error, NonStreamingError::UndecodedForm)),
            Ok(_response) => panic!("a multipart request without its form must not be sent"),
        }
        assert_eq!(captured_count(&seen), 0);
    }
}
