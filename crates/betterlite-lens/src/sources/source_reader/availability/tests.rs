#![cfg(test)]

use crate::models::settings::Scope;
use crate::sources::activity_availability::ActivityAvailability;
use crate::sources::source_reader::SourceReader;
use crate::sources::stub_storage::StubStorage;

#[tokio::test]
async fn returns_the_first_row() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(br#"[{"traces": true, "requests": false}]"#.to_vec()),
    };
    let availability = reader.availability(&Scope::default()).await.expect("ok");
    assert_eq!(
        availability,
        ActivityAvailability {
            traces: true,
            requests: false,
        }
    );
}

#[tokio::test]
async fn returns_the_default_when_storage_is_empty() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"[]".to_vec()),
    };
    let availability = reader.availability(&Scope::default()).await.expect("ok");
    assert_eq!(availability, ActivityAvailability::default());
}

#[tokio::test]
async fn propagates_a_decode_error() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"not json".to_vec()),
    };
    let result = reader.availability(&Scope::default()).await;
    result.unwrap_err();
}
