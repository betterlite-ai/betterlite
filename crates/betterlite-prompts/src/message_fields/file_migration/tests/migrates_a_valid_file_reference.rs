use super::*;
use rstest::rstest;

#[rstest]
#[case::file_id_wins_when_both_are_set(
    Object::from_iter([
        ("file_id".to_owned(), Json::Text("file-123".to_owned())),
        ("file_data".to_owned(), Json::Text("base64data".to_owned())),
    ]),
    "file-123",
    None
)]
#[case::falls_back_to_file_data_when_file_id_is_absent(
    Object::from_iter([("file_data".to_owned(), Json::Text("base64data".to_owned()))]),
    "base64data",
    None
)]
#[case::carries_format_into_the_migrated_image_url(
    Object::from_iter([
        ("file_id".to_owned(), Json::Text("file-123".to_owned())),
        ("format".to_owned(), Json::Text("image/png".to_owned())),
    ]),
    "file-123",
    Some("image/png")
)]
fn message_fields_behavior_tests_behavior(
    #[case] file: Object,
    #[case] expected_url: &str,
    #[case] expected_format: Option<&str>,
) {
    let part = file_part(file);
    let migrated = migrate_file_to_image_url(&part).expect("a usable file reference migrates");
    assert_eq!(migrated.kind, "image_url");
    let Some(Json::Object(image_url)) = migrated.fields.get("image_url") else {
        panic!("migrated part must carry an image_url object");
    };
    assert_eq!(
        image_url.get("url").and_then(Json::as_str),
        Some(expected_url)
    );
    assert_eq!(
        image_url.get("format").and_then(Json::as_str),
        expected_format
    );
}
