use async_trait::async_trait;
use thiserror::Error;

use super::super::store::{RagStore, StoreRequest, StoreResult};
use super::*;

#[derive(Debug, Error)]
enum StoreFailure {
    #[error("the store refused")]
    Refused,
}

struct FakeStore {
    chunks: Mutex<usize>,
}

#[async_trait]
impl RagStore for FakeStore {
    type Error = StoreFailure;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error> {
        *self.chunks.lock().unwrap() = request.chunks.len();
        if request.existing_file_id == Some("fail") {
            return Err(StoreFailure::Refused);
        }
        Ok((
            request.filename.map(str::to_owned),
            request.existing_file_id.map(str::to_owned),
        ))
    }
}

#[tokio::test]
async fn store_contract_returns_the_vector_store_and_file_ids() {
    let store = FakeStore {
        chunks: Mutex::new(0),
    };
    let chunks = vec!["one".to_owned(), "two".to_owned()];
    let result = store
        .store(StoreRequest {
            file_content: Some(b"content"),
            filename: Some("doc.txt"),
            content_type: Some("text/plain"),
            chunks: &chunks,
            embeddings: None,
            existing_file_id: Some("file_1"),
        })
        .await
        .expect("the store succeeds");
    assert_eq!(
        result,
        (Some("doc.txt".to_owned()), Some("file_1".to_owned()))
    );
    assert_eq!(*store.chunks.lock().unwrap(), 2);
}

#[tokio::test]
async fn store_contract_propagates_the_provider_error() {
    let store = FakeStore {
        chunks: Mutex::new(0),
    };
    let chunks = Vec::new();
    let error = store
        .store(StoreRequest {
            file_content: None,
            filename: None,
            content_type: None,
            chunks: &chunks,
            embeddings: None,
            existing_file_id: Some("fail"),
        })
        .await
        .expect_err("the store fails");
    assert!(matches!(error, StoreFailure::Refused));
}
