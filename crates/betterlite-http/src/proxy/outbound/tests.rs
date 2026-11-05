#![cfg(test)]

use std::time::Duration;

use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use rstest::rstest;

use super::{OutboundRequest, RequestBody, merge_query};
use crate::proxy::multipart::MultipartPart;
use crate::proxy::tests::{default_client, kind, reply, serve};

fn part(name: &str, data: &[u8]) -> MultipartPart {
    MultipartPart {
        name: name.to_owned(),
        file_name: Some("a.txt".to_owned()),
        content_type: Some("text/plain".to_owned()),
        data: data.to_vec(),
    }
}

fn multipart(parts: Vec<MultipartPart>) -> RequestBody {
    RequestBody::Multipart {
        boundary: "b".to_owned(),
        parts,
    }
}

const FORM: &str = "--b\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nhi\r\n--b--\r\n";

#[rstest]
#[case::json_gets_its_content_type(Method::POST, "", &[], RequestBody::Json(b"{}".to_vec()), "POST /|application/json|{}")]
#[case::bytes_keep_their_content_type(Method::PUT, "", &[], RequestBody::Bytes { content_type: "text/plain".parse().unwrap(), data: b"raw".to_vec() }, "PUT /|text/plain|raw")]
#[case::empty_body_has_no_content_type(Method::GET, "", &[], RequestBody::Empty, "GET /|none|")]
#[case::multipart_is_serialized_with_its_boundary(Method::POST, "", &[], multipart(vec![part("f", b"hi")]), &format!("POST /|multipart/form-data; boundary=b|{FORM}"))]
#[case::multipart_name_with_a_quote_is_refused(Method::POST, "", &[], multipart(vec![part("f\"x", b"hi")]), "MultipartName")]
#[case::multipart_data_holding_the_boundary_is_refused(Method::POST, "", &[], multipart(vec![part("f", b"--b")]), "MultipartBoundary")]
#[case::explicit_query_is_appended(Method::GET, "a=1", &[("b", "2")], RequestBody::Empty, "GET /?a=1&b=2|none|")]
#[case::url_query_wins_a_collision(Method::GET, "a=1", &[("a", "2"), ("b", "3")], RequestBody::Empty, "GET /?a=1&b=3|none|")]
#[case::repeats_in_the_url_keep_their_order(Method::GET, "a=1&a=2", &[("a", "9")], RequestBody::Empty, "GET /?a=1&a=2|none|")]
#[case::repeats_in_explicit_params_keep_their_order(Method::GET, "", &[("k", "1"), ("k", "2")], RequestBody::Empty, "GET /?k=1&k=2|none|")]
#[tokio::test]
async fn http_pool_tests_behavior(
    #[case] method: Method,
    #[case] url_query: &str,
    #[case] query: &[(&str, &str)],
    #[case] body: RequestBody,
    #[case] expected: &str,
) {
    let server = serve(reply(&["200 OK", "content-length: 0"], b""), Duration::ZERO).await;
    let mut url: Url = server.base.clone();
    if !url_query.is_empty() {
        url.set_query(Some(url_query));
    }
    let request = OutboundRequest {
        method,
        url,
        query: query
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        headers: HeaderMap::new(),
        body,
        deadline: Duration::from_secs(5),
    };
    let observed = match default_client().send_buffered(request).await {
        Ok(_response) => {
            let sent = server.heads.lock().unwrap().remove(0);
            let (head, sent_body) = sent.split_once("\r\n\r\n").unwrap();
            let line = head.lines().next().unwrap().trim_end_matches(" HTTP/1.1");
            let content_type = head
                .lines()
                .find_map(|header| header.strip_prefix("content-type: "))
                .unwrap_or("none");
            format!("{line}|{content_type}|{sent_body}")
        }
        Err(error) => kind(&error),
    };
    assert_eq!(observed, expected);
}

#[test]
fn merge_query_leaves_a_url_without_explicit_params_untouched() {
    let mut url = Url::parse("http://h/p?a=%20&a=2").unwrap();
    merge_query(&mut url, &[]);
    assert_eq!(url.as_str(), "http://h/p?a=%20&a=2");
}
