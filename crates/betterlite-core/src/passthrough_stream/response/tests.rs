#![cfg(test)]

use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::stream;

use super::{AsyncChunkSource as _, StreamBody};

fn body_of(items: Vec<Result<Vec<u8>, HttpError>>, body_limit: usize) -> StreamBody {
    StreamBody {
        chunks: stream::iter(items).boxed(),
        body_limit,
    }
}

#[tokio::test]
async fn next_chunk_passes_the_stream_through() {
    let mut body = body_of(vec![Ok(b"one".to_vec()), Ok(b"two".to_vec())], 16);
    assert_eq!(
        body.next_chunk().await.expect("first"),
        Some(b"one".to_vec())
    );
    assert_eq!(
        body.next_chunk().await.expect("second"),
        Some(b"two".to_vec())
    );
    assert_eq!(body.next_chunk().await.expect("end"), None);
}

#[tokio::test]
async fn read_body_concatenates_every_chunk() {
    let mut body = body_of(vec![Ok(b"one".to_vec()), Ok(b"two".to_vec())], 6);
    assert_eq!(body.read_body().await.expect("fits"), b"onetwo".to_vec());
}

#[tokio::test]
async fn read_body_refuses_a_body_over_the_limit() {
    let mut body = body_of(vec![Ok(b"one".to_vec()), Ok(b"two".to_vec())], 5);
    let error = body.read_body().await.expect_err("six bytes is over five");
    assert!(matches!(error, HttpError::TooLarge { limit: 5 }));
}

#[tokio::test]
async fn read_body_accepts_a_body_at_the_limit() {
    let mut body = body_of(vec![Ok(b"one".to_vec()), Ok(b"two".to_vec())], 6);
    assert_eq!(
        body.read_body().await.expect("six bytes fits six"),
        b"onetwo".to_vec()
    );
}

#[tokio::test]
async fn read_body_propagates_a_stream_error() {
    let mut body = body_of(vec![Ok(b"one".to_vec()), Err(HttpError::Timeout)], 16);
    let error = body.read_body().await.expect_err("the stream fails");
    assert!(matches!(error, HttpError::Timeout));
}

#[tokio::test]
async fn close_ends_the_stream_and_the_body() {
    let mut body = body_of(vec![Ok(b"one".to_vec())], 16);
    body.close().await.expect("the close succeeds");
    assert_eq!(body.next_chunk().await.expect("closed"), None);
    assert!(body.read_body().await.expect("closed").is_empty());
}
