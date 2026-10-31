use super::*;
use rstest::rstest;

#[rstest]
#[case::base64(
    Object::from_iter([
        ("type".to_owned(), Json::Text("base64".to_owned())),
        ("media_type".to_owned(), Json::Text("image/png".to_owned())),
        ("data".to_owned(), Json::Text("abc".to_owned())),
    ]),
    Some("data:image/png;base64,abc")
)]
#[case::base64_default_media_type(
    Object::from_iter([
        ("type".to_owned(), Json::Text("base64".to_owned())),
        ("data".to_owned(), Json::Text("abc".to_owned())),
    ]),
    Some("data:image/jpeg;base64,abc")
)]
#[case::base64_empty_data(
    Object::from_iter([
        ("type".to_owned(), Json::Text("base64".to_owned())),
        ("data".to_owned(), Json::Text(String::new())),
    ]),
    None
)]
#[case::url(
    Object::from_iter([
        ("type".to_owned(), Json::Text("url".to_owned())),
        ("url".to_owned(), Json::Text("https://img".to_owned())),
    ]),
    Some("https://img")
)]
#[case::url_missing(
    Object::from_iter([("type".to_owned(), Json::Text("url".to_owned()))]),
    Some("")
)]
#[case::unknown(
    Object::from_iter([("type".to_owned(), Json::Text("other".to_owned()))]),
    None
)]
fn message_content_behavior_tests_behavior(#[case] source: Object, #[case] expected: Option<&str>) {
    assert_eq!(
        anthropic_image_source_url(&source),
        expected.map(str::to_owned)
    );
}
