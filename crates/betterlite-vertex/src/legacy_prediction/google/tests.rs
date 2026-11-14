#![cfg(test)]

use std::collections::VecDeque;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use futures_util::{StreamExt as _, stream};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, Method};
use secrecy::SecretString;

use super::{PredictionChunks, auth_headers, message_of, rpc_request, rpc_url, success_body};
use crate::legacy_prediction::error::PredictionError;
use crate::legacy_prediction::frames::PredictionFrames;
use crate::legacy_prediction::records::{ChunkKind, LegacyChunk};

const RESOURCE: &str = "projects/p/locations/l/publishers/google/models/m";

fn response(status: u16, body: &[u8]) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.to_vec(),
    }
}

fn chunks(reads: Vec<Result<Vec<u8>, HttpError>>) -> PredictionChunks {
    PredictionChunks {
        body: stream::iter(reads).boxed(),
        frames: PredictionFrames::default(),
        kind: ChunkKind::Text,
        pending: VecDeque::new(),
        finished: false,
    }
}

#[test]
fn google_rpc_url_names_the_regional_host() {
    assert_eq!(
        rpc_url("us-central1", RESOURCE, "predict")
            .expect("a url")
            .as_str(),
        "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/l/publishers/google/models/m:predict"
    );
    assert_eq!(
        rpc_url("global", "x", "serverStreamingPredict")
            .expect("a url")
            .as_str(),
        "https://aiplatform.googleapis.com/v1/x:serverStreamingPredict"
    );
    assert!(matches!(
        rpc_url("a/b", RESOURCE, "predict"),
        Err(PredictionError::VertexUrl { .. })
    ));
}

#[test]
fn google_rpc_request_carries_the_bearer_token() {
    let token = SecretString::from("tok");
    let url = rpc_url("global", "x", "predict").expect("a url");
    let request = rpc_request(&token, url.clone(), b"{}".to_vec(), Duration::from_secs(7))
        .expect("a request");
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url, url);
    assert!(request.query.is_empty());
    assert_eq!(request.deadline, Duration::from_secs(7));
    assert!(matches!(&request.body, RequestBody::Json(bytes) if bytes == b"{}"));
    assert_eq!(request.headers[CONTENT_TYPE], "application/json");
    assert_eq!(request.headers[AUTHORIZATION], "Bearer tok");
    assert!(request.headers[AUTHORIZATION].is_sensitive());
    let headers = auth_headers(&token).expect("headers");
    assert_eq!(headers[AUTHORIZATION], "Bearer tok");
    assert!(matches!(
        auth_headers(&SecretString::from("bad\ntoken")),
        Err(PredictionError::Header { .. })
    ));
}

#[test]
fn google_success_body_passes_only_2xx() {
    assert_eq!(success_body(response(200, b"ok")).expect("200"), b"ok");
    assert_eq!(success_body(response(299, b"ok")).expect("299"), b"ok");
    for status in [199, 300, 404, 500] {
        let Err(PredictionError::Http {
            status: seen,
            message,
        }) = success_body(response(status, b"nope"))
        else {
            panic!("status {status} is an error");
        };
        assert_eq!(seen, status);
        assert_eq!(message, "nope");
    }
}

#[test]
fn google_message_of_names_binary_bodies() {
    assert_eq!(message_of(b"quota"), "quota");
    assert_eq!(message_of(&[0xff, 0xfe]), "2 bytes that are not UTF-8");
}

#[tokio::test]
async fn google_chunks_split_the_array_across_reads() {
    let first = br#"[{"outputs":[{"structVal":{"content":{"stringVal":["a"]}}}]},{"outp"#;
    let second = br#"uts":[{"structVal":{"content":{"stringVal":["b"]}}}]}]"#;
    let seen: Vec<_> = chunks(vec![Ok(first.to_vec()), Ok(second.to_vec())])
        .collect()
        .await;
    let texts: Vec<_> = seen
        .into_iter()
        .map(|item| item.expect("a chunk"))
        .collect();
    assert_eq!(
        texts,
        vec![
            LegacyChunk {
                text: String::from("a")
            },
            LegacyChunk {
                text: String::from("b")
            },
        ]
    );
}

#[tokio::test]
async fn google_chunks_report_truncation_and_read_errors() {
    let mut truncated = chunks(vec![Ok(br#"[{"outputs":[]"#.to_vec())]);
    assert!(matches!(
        truncated.next().await,
        Some(Err(PredictionError::StreamUnfinished))
    ));
    assert!(truncated.next().await.is_none());
    let mut failed = chunks(vec![Err(HttpError::Timeout), Ok(b"[]".to_vec())]);
    assert!(matches!(
        failed.next().await,
        Some(Err(PredictionError::Read { .. }))
    ));
    assert!(failed.next().await.is_none());
    let mut shaped = chunks(vec![Ok(b"{}".to_vec())]);
    assert!(matches!(
        shaped.next().await,
        Some(Err(PredictionError::StreamShape))
    ));
    assert!(shaped.next().await.is_none());
}
