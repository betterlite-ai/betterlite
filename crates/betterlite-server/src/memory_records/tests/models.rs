use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::memory_records::models::{
    CreateRequest, DeleteResponse, ListResponse, MemoryRow, UpdateRequest,
};

#[test]
fn all_ten_row_fields() {
    let text = r#"{"memory_id":"mem-1","key":"alpha","value":"hello","metadata":{"source":"test"},"user_id":"alice","team_id":"team-a","created_at":"2026-01-02T03:04:05","created_by":"alice","updated_at":"2026-01-03T04:05:06","updated_by":"bob"}"#;
    let stored: MemoryRow = serde_json::from_str(text).unwrap();
    assert_eq!(stored.memory_id, "mem-1");
    assert_eq!(stored.key, "alpha");
    assert_eq!(stored.value, "hello");
    assert_eq!(
        stored.metadata,
        Some(JsonData::Object(BTreeMap::from([(
            "source".to_owned(),
            JsonData::String("test".to_owned())
        )])))
    );
    assert_eq!(stored.user_id.as_deref(), Some("alice"));
    assert_eq!(stored.team_id.as_deref(), Some("team-a"));
    assert_eq!(
        stored.created_at.unwrap().utc().to_rfc3339(),
        "2026-01-02T03:04:05+00:00"
    );
    assert_eq!(stored.created_by.as_deref(), Some("alice"));
    assert_eq!(
        stored.updated_at.unwrap().utc().to_rfc3339(),
        "2026-01-03T04:05:06+00:00"
    );
    assert_eq!(stored.updated_by.as_deref(), Some("bob"));
    assert_eq!(serde_json::to_string(&stored).unwrap(), text);
}

#[test]
fn create_request_required_and_optional() {
    let minimal: CreateRequest = serde_json::from_str(r#"{"key":"note","value":"hello"}"#).unwrap();
    assert_eq!(minimal.metadata, None);
    assert_eq!(minimal.user_id, None);
    assert_eq!(minimal.team_id, None);
    assert_eq!(minimal.metadata_column().unwrap(), None);

    let full: CreateRequest = serde_json::from_str(
        r#"{"key":"note","value":"hello","metadata":{"tag":"x"},"user_id":"alice","team_id":"team-a"}"#,
    )
    .unwrap();
    assert_eq!(full.user_id.as_deref(), Some("alice"));
    assert_eq!(full.team_id.as_deref(), Some("team-a"));
    assert_eq!(
        full.metadata_column().unwrap().as_deref(),
        Some(r#"{"tag":"x"}"#)
    );

    serde_json::from_str::<CreateRequest>(r#"{"key":"note"}"#).unwrap_err();
    serde_json::from_str::<CreateRequest>(r#"{"value":"hello"}"#).unwrap_err();
    serde_json::from_str::<CreateRequest>(r#"{"key":7,"value":"hello"}"#).unwrap_err();
}

#[test]
fn update_request_field_presence() {
    let empty: UpdateRequest = serde_json::from_str("{}").unwrap();
    assert_eq!(empty.value(), None);
    assert!(empty.fields_sent().is_empty());

    let value_only: UpdateRequest = serde_json::from_str(r#"{"value":"hello"}"#).unwrap();
    assert_eq!(value_only.value(), Some("hello"));
    assert_eq!(value_only.fields_sent(), vec!["value"]);

    let explicit_null: UpdateRequest = serde_json::from_str(r#"{"value":null}"#).unwrap();
    assert_eq!(explicit_null.value(), None);
    assert_eq!(explicit_null.fields_sent(), vec!["value"]);

    let metadata_null: UpdateRequest = serde_json::from_str(r#"{"metadata":null}"#).unwrap();
    assert_eq!(metadata_null.metadata, Some(None));
    assert_eq!(metadata_null.fields_sent(), vec!["metadata"]);

    let scope_null: UpdateRequest =
        serde_json::from_str(r#"{"user_id":null,"team_id":null}"#).unwrap();
    assert_eq!(scope_null.fields_sent(), vec!["team_id", "user_id"]);

    let everything: UpdateRequest = serde_json::from_str(
        r#"{"value":"hello","metadata":{"tag":"x"},"user_id":"alice","team_id":"team-a"}"#,
    )
    .unwrap();
    assert_eq!(
        everything.fields_sent(),
        vec!["metadata", "team_id", "user_id", "value"]
    );
}

#[test]
fn list_response_structure() {
    let text = r#"{"memories":[{"memory_id":"mem-1","key":"note","value":"hello","metadata":null,"user_id":"alice","team_id":null,"created_at":null,"created_by":"alice","updated_at":null,"updated_by":"alice"}],"total":1}"#;
    let listing: ListResponse = serde_json::from_str(text).unwrap();
    assert_eq!(listing.total, 1);
    assert_eq!(listing.memories.len(), 1);
    assert_eq!(listing.memories[0].memory_id, "mem-1");
    assert_eq!(serde_json::to_string(&listing).unwrap(), text);

    let empty: ListResponse = serde_json::from_str(r#"{"memories":[],"total":0}"#).unwrap();
    assert!(empty.memories.is_empty());
    assert_eq!(empty.total, 0);
}

#[test]
fn delete_response_structure() {
    let text = r#"{"key":"note","deleted":true}"#;
    let deleted: DeleteResponse = serde_json::from_str(text).unwrap();
    assert_eq!(deleted.key, "note");
    assert!(deleted.deleted);
    assert_eq!(serde_json::to_string(&deleted).unwrap(), text);

    let kept: DeleteResponse = serde_json::from_str(r#"{"key":"note","deleted":false}"#).unwrap();
    assert!(!kept.deleted);
}

fn create_with_metadata(metadata: JsonData) -> CreateRequest {
    CreateRequest {
        key: "note".to_owned(),
        value: "hello".to_owned(),
        metadata: Some(metadata),
        user_id: None,
        team_id: None,
    }
}

#[test]
fn metadata_json_serialization() {
    let value = JsonData::Object(BTreeMap::from([
        (
            "tags".to_owned(),
            JsonData::Array(vec![JsonData::String("a".to_owned()), JsonData::Null]),
        ),
        ("count".to_owned(), JsonData::Bool(true)),
    ]));
    let object_request = create_with_metadata(value);
    let column = object_request.metadata_column().unwrap();
    assert_eq!(
        column.as_deref(),
        Some(r#"{"count":true,"tags":["a",null]}"#)
    );
    let decoded: JsonData = serde_json::from_str(column.as_deref().unwrap()).unwrap();
    assert_eq!(decoded, object_request.metadata.clone().unwrap());

    let scalar_request = create_with_metadata(JsonData::String("hello".to_owned()));
    assert_eq!(
        scalar_request.metadata_column().unwrap().as_deref(),
        Some(r#""hello""#)
    );

    let number_request = create_with_metadata(JsonData::Number(serde_json::Number::from(3_i64)));
    assert_eq!(
        number_request.metadata_column().unwrap().as_deref(),
        Some("3")
    );
}

#[test]
fn null_vs_absent_metadata() {
    let absent_create: CreateRequest =
        serde_json::from_str(r#"{"key":"note","value":"hello"}"#).unwrap();
    let null_create: CreateRequest =
        serde_json::from_str(r#"{"key":"note","value":"hello","metadata":null}"#).unwrap();
    assert_eq!(absent_create.metadata, None);
    assert_eq!(null_create.metadata, None);
    assert_eq!(absent_create.metadata_column().unwrap(), None);
    assert_eq!(null_create.metadata_column().unwrap(), None);

    let absent_update: UpdateRequest = serde_json::from_str(r#"{"value":"hello"}"#).unwrap();
    let null_update: UpdateRequest =
        serde_json::from_str(r#"{"value":"hello","metadata":null}"#).unwrap();
    let present_update: UpdateRequest =
        serde_json::from_str(r#"{"value":"hello","metadata":{"tag":"x"}}"#).unwrap();
    assert_eq!(absent_update.metadata_column().unwrap(), None);
    assert_eq!(
        null_update.metadata_column().unwrap().as_deref(),
        Some("null")
    );
    assert_eq!(
        present_update.metadata_column().unwrap().as_deref(),
        Some(r#"{"tag":"x"}"#)
    );

    let row_with_null = r#"{"memory_id":"mem-1","key":"note","value":"hello","metadata":null}"#;
    let row_without = r#"{"memory_id":"mem-1","key":"note","value":"hello"}"#;
    assert_eq!(
        serde_json::from_str::<MemoryRow>(row_with_null)
            .unwrap()
            .metadata,
        None
    );
    assert_eq!(
        serde_json::from_str::<MemoryRow>(row_without)
            .unwrap()
            .metadata,
        None
    );
}
