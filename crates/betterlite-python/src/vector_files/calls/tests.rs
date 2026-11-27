#![cfg(test)]

use crate::vector_files::convert::{CreateWire, ListWire, parse};
use crate::vector_files::write_calls::WriteSide;

use super::{FileSide, ReadSide};

#[test]
fn a_side_carries_the_call_name_and_provider_its_wire_gave() {
    assert_eq!(ReadSide::Retrieve.call_type(), "retrieve");
    assert_eq!(ReadSide::Content.call_type(), "retrieve_content");
    let creating: CreateWire = parse(
        r#"{"vector_store_id":"vs_a","file_id":"file_1","custom_llm_provider":"azure","kwargs":{}}"#,
        0,
    )
    .unwrap();
    assert_eq!(FileSide::Create(&creating).call_type(), "create");
    assert_eq!(FileSide::Create(&creating).provider(), Some("azure"));
    let listing: ListWire = parse(
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","kwargs":{}}"#,
        0,
    )
    .unwrap();
    assert_eq!(FileSide::List(&listing).call_type(), "list");
    assert_eq!(FileSide::List(&listing).provider(), Some("openai"));
    assert_eq!(WriteSide::Update.call_type(), "update");
    assert_eq!(WriteSide::Delete.call_type(), "delete");
}
