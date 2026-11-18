use super::support::parse;
use crate::team_request_contracts::metadata::{
    TeamMetadataFieldSchema, TeamMetadataSchemaResponse,
};
use crate::team_request_contracts::shared::to_data;

#[test]
fn metadata_key_and_required_fields() {
    let field = TeamMetadataFieldSchema::from_value(&parse(r#"{"key": "k"}"#)).unwrap();
    assert_eq!(field.key, "k");
    assert_eq!(field.label, None);
    assert_eq!(
        to_data(&field).unwrap(),
        parse(r#"{"key": "k", "label": null}"#)
    );
    let labeled =
        TeamMetadataFieldSchema::from_value(&parse(r#"{"key": " ", "label": "L"}"#)).unwrap();
    assert_eq!(labeled.key, " ");
    assert_eq!(labeled.label.as_deref(), Some("L"));
    assert_eq!(
        to_data(&labeled).unwrap(),
        parse(r#"{"key": " ", "label": "L"}"#)
    );
    let empty = TeamMetadataFieldSchema::from_value(&parse(r#"{"key": ""}"#)).unwrap_err();
    assert_eq!(empty.to_string(), "String should have at least 1 character");
    assert!(TeamMetadataFieldSchema::from_value(&parse(r#"{"label": "L"}"#)).is_err());
    assert!(TeamMetadataFieldSchema::from_value(&parse(r#"{"key": 1}"#)).is_err());
    assert!(TeamMetadataFieldSchema::from_value(&parse(r#"{"key": null}"#)).is_err());

    let empty_schema = TeamMetadataSchemaResponse::from_value(&parse(r#"{"fields": []}"#)).unwrap();
    assert_eq!(empty_schema.fields, Vec::new());
    let schema = TeamMetadataSchemaResponse::from_value(&parse(
        r#"{"fields": [{"key": "a"}, {"key": "b", "label": null}]}"#,
    ))
    .unwrap();
    assert_eq!(schema.fields.len(), 2);
    assert_eq!(schema.fields[0].key, "a");
    assert_eq!(schema.fields[1].key, "b");
    assert_eq!(
        to_data(&schema).unwrap(),
        parse(r#"{"fields": [{"key": "a", "label": null}, {"key": "b", "label": null}]}"#)
    );
    assert!(TeamMetadataSchemaResponse::from_value(&parse("{}")).is_err());
    assert!(TeamMetadataSchemaResponse::from_value(&parse(r#"{"fields": null}"#)).is_err());
    assert!(
        TeamMetadataSchemaResponse::from_value(&parse(r#"{"fields": [{"key": ""}]}"#)).is_err()
    );
    assert!(
        TeamMetadataSchemaResponse::from_value(&parse(r#"{"fields": [{"key": "a", "x": 1}]}"#))
            .is_err()
    );
}
