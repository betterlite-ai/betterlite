#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::api::vector_management_write::rows::{
    FieldChange, VectorStoreChanges, insert_sql, row_record,
};

#[test]
fn insert_sql_names_every_column_and_the_database_clock() {
    let mut document = BTreeMap::new();
    document.insert(
        "vector_store_id".to_owned(),
        JsonData::String("vs_a".to_owned()),
    );
    document.insert(
        "custom_llm_provider".to_owned(),
        JsonData::String("openai".to_owned()),
    );
    let expected = concat!(
        "INSERT INTO \"litellm_managedvectorstorestable\" AS t ",
        "(\"custom_llm_provider\", \"vector_store_id\", \"updated_at\") ",
        "SELECT r.\"custom_llm_provider\", r.\"vector_store_id\", now() ",
        "FROM jsonb_populate_record(NULL::\"litellm_managedvectorstorestable\", $1::text::jsonb) ",
        "AS r RETURNING row_to_json(t)::text"
    );
    assert_eq!(expected, insert_sql(&document).as_str());
}

#[test]
fn row_record_decodes_the_written_row() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "vector_store_id".to_owned(),
        JsonData::String("vs_a".to_owned()),
    );
    fields.insert(
        "custom_llm_provider".to_owned(),
        JsonData::String("openai".to_owned()),
    );
    let decoded = row_record(Some(JsonData::Object(fields)))
        .expect("the returned row decodes")
        .expect("the row is present");
    assert_eq!(Some("vs_a".to_owned()), decoded.vector_store_id);
    assert!(
        row_record(None)
            .expect("an absent row stays absent")
            .is_none()
    );
}

#[test]
fn changes_document_writes_only_the_present_fields() {
    let changes = VectorStoreChanges {
        custom_llm_provider: FieldChange::Value("openai".to_owned()),
        vector_store_name: FieldChange::Null,
        vector_store_description: FieldChange::Omitted,
        vector_store_metadata: FieldChange::Value("{}".to_owned()),
    };
    let mut expected = BTreeMap::new();
    expected.insert(
        "custom_llm_provider".to_owned(),
        JsonData::String("openai".to_owned()),
    );
    expected.insert("vector_store_name".to_owned(), JsonData::Null);
    expected.insert(
        "vector_store_metadata".to_owned(),
        JsonData::String("{}".to_owned()),
    );
    assert_eq!(expected, changes.document());
}
