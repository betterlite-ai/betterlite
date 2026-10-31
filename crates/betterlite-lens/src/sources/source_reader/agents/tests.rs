#![cfg(test)]

use crate::models::settings::Scope;
use crate::sources::source_reader::SourceReader;
use crate::sources::stub_storage::StubStorage;

#[tokio::test]
async fn projects_agent_names_from_every_row() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(
            br#"[{"agent_name": "reviewer"}, {"agent_name": "writer"}]"#.to_vec(),
        ),
    };
    let agents = reader.agents(&Scope::default()).await.expect("ok");
    assert_eq!(agents, vec!["reviewer".to_owned(), "writer".to_owned()]);
}

#[tokio::test]
async fn returns_an_empty_list_when_storage_is_empty() {
    let reader = SourceReader {
        storage: StubStorage::with_bytes(b"[]".to_vec()),
    };
    let agents = reader.agents(&Scope::default()).await.expect("ok");
    assert!(agents.is_empty());
}
