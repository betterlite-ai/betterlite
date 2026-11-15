#![cfg(test)]

use std::collections::VecDeque;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Extra;
use futures_util::{StreamExt as _, stream};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, Method};
use rstest::rstest;
use secrecy::SecretString;

use super::{
    GOOG_USER_PROJECT, PredictionChunks, RpcAuth, auth_headers, endpoint_auth, message_of,
    publisher_auth, rpc_request, rpc_url, success_body,
};
use crate::legacy_prediction::client::{EndpointPredictRequest, PublisherPredictRequest};
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
    let auth = RpcAuth {
        token: &token,
        project: "proj",
    };
    let url = rpc_url("global", "x", "predict").expect("a url");
    let request =
        rpc_request(auth, url, b"{}".to_vec(), Duration::from_secs(7)).expect("a request");
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.headers[AUTHORIZATION], "Bearer tok");
    assert!(request.headers[AUTHORIZATION].is_sensitive());
    assert_eq!(request.headers[GOOG_USER_PROJECT], "proj");
    assert!(request.headers[GOOG_USER_PROJECT].is_sensitive());
}

#[test]
fn google_rpc_request_encodes_the_body_and_deadline() {
    let token = SecretString::from("tok");
    let auth = RpcAuth {
        token: &token,
        project: "proj",
    };
    let url = rpc_url("global", "x", "predict").expect("a url");
    let request =
        rpc_request(auth, url.clone(), b"{}".to_vec(), Duration::from_secs(7)).expect("a request");
    assert_eq!(request.url, url);
    assert!(request.query.is_empty());
    assert_eq!(request.deadline, Duration::from_secs(7));
    assert_eq!(request.headers[CONTENT_TYPE], "application/json");
    assert!(matches!(&request.body, RequestBody::Json(bytes) if bytes == b"{}"));
}

#[test]
fn google_auth_headers_reject_control_characters() {
    let token = SecretString::from("tok");
    assert!(matches!(
        auth_headers(RpcAuth {
            token: &SecretString::from("bad\ntoken"),
            project: "proj",
        }),
        Err(PredictionError::Header { .. })
    ));
    assert!(matches!(
        auth_headers(RpcAuth {
            token: &token,
            project: "bad\nproject",
        }),
        Err(PredictionError::Header { .. })
    ));
}

#[test]
fn google_predict_requests_carry_the_quota_project() {
    let publisher = PublisherPredictRequest {
        token: SecretString::from("tok"),
        project: String::from("publisher-project"),
        location: String::from("us-central1"),
        model: String::from("chat-bison"),
        prompt: String::from("hello"),
        options: Extra::new(),
    };
    let headers = auth_headers(publisher_auth(&publisher)).expect("publisher headers");
    assert_eq!(headers[AUTHORIZATION], "Bearer tok");
    assert_eq!(headers[GOOG_USER_PROJECT], "publisher-project");
    let endpoint = EndpointPredictRequest {
        token: SecretString::from("tok"),
        project: String::from("endpoint-project"),
        location: String::from("us-central1"),
        endpoint: String::from("projects/p/locations/l/endpoints/e"),
        instances: Vec::new(),
    };
    let endpoint_headers = auth_headers(endpoint_auth(&endpoint)).expect("endpoint headers");
    assert_eq!(endpoint_headers[AUTHORIZATION], "Bearer tok");
    assert_eq!(endpoint_headers[GOOG_USER_PROJECT], "endpoint-project");
}

#[rstest]
#[case(199)]
#[case(300)]
#[case(404)]
#[case(500)]
fn google_success_body_passes_only_2xx(#[case] status: u16) {
    assert_eq!(success_body(response(200, b"ok")).expect("200"), b"ok");
    assert_eq!(success_body(response(299, b"ok")).expect("299"), b"ok");
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
