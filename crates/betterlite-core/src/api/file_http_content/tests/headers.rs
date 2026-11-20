use http::header::HeaderValue;
use rstest::rstest;

use super::support::http_headers;
use crate::api::file_http_content::service::decoded_body_headers;

#[rstest]
#[case::absent(None)]
#[case::identity(Some("identity"))]
#[case::upper(Some("IDENTITY"))]
#[case::mixed(Some("Identity"))]
fn decoded_headers_keep_an_identity_encoding(#[case] encoding: Option<&str>) {
    let mut headers = http_headers(&[
        ("content-type", "application/octet-stream"),
        ("content-disposition", "attachment; filename=\"a.bin\""),
        ("x-repeat", "one"),
    ]);
    headers.append("x-repeat", HeaderValue::from_static("two"));
    if let Some(value) = encoding {
        headers.insert(
            "content-encoding",
            HeaderValue::from_str(value).expect("the header value"),
        );
    }
    assert_eq!(decoded_body_headers(&headers), headers);
}

#[rstest]
#[case::gzip(&[("content-encoding", "gzip")])]
#[case::empty(&[("content-encoding", "")])]
#[case::repeated(&[
    ("content-encoding", "identity"),
    ("content-encoding", "identity"),
])]
fn decoded_headers_drop_stale_transfer_headers(#[case] stale: &[(&'static str, &'static str)]) {
    let mut pairs: Vec<(&'static str, &'static str)> = stale.to_vec();
    pairs.push(("content-length", "10"));
    pairs.push(("content-disposition", "attachment"));
    pairs.push(("content-type", "application/octet-stream"));
    pairs.push(("x-repeat", "one"));
    let mut headers = http_headers(&pairs);
    headers.append("content-length", HeaderValue::from_static("11"));
    headers.append("x-repeat", HeaderValue::from_static("two"));
    let decoded = decoded_body_headers(&headers);
    assert!(decoded.get("content-encoding").is_none());
    assert!(decoded.get("content-length").is_none());
    assert_eq!(
        decoded
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        Some("attachment")
    );
    assert_eq!(
        decoded
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    let repeats: Vec<&str> = decoded
        .get_all("x-repeat")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(repeats, vec!["one", "two"]);
    assert_eq!(headers.get_all("content-length").iter().count(), 2);
}
