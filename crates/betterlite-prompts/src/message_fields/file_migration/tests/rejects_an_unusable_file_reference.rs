use super::*;
use rstest::rstest;

#[rstest]
#[case::missing_file_field(
    ContentPart { kind: "file".to_owned(), fields: Object::new() },
    FileMigrationError::MissingFile
)]
#[case::file_field_is_not_an_object(
    ContentPart {
        kind: "file".to_owned(),
        fields: Object::from_iter([("file".to_owned(), Json::Text("not-a-dict".to_owned()))]),
    },
    FileMigrationError::MissingFile
)]
#[case::file_id_and_file_data_are_both_missing(
    file_part(Object::new()),
    FileMigrationError::MissingFileReference
)]
fn message_fields_behavior_tests_behavior(
    #[case] part: ContentPart,
    #[case] expected_error: FileMigrationError,
) {
    assert_eq!(migrate_file_to_image_url(&part), Err(expected_error));
}
