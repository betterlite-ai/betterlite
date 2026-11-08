use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{environment, transformation_json};
use crate::argilla_logging::settings::{
    ArgillaSettings, ConstructorArguments, PayloadField, SettingsError,
};

fn build(value: &Json) -> Result<ArgillaSettings, SettingsError> {
    ArgillaSettings::from_environment(
        &environment(&[("ARGILLA_API_KEY", "key")]),
        &ConstructorArguments::default(),
        Some(value),
    )
}

#[test]
fn rejects_a_missing_transformation_object() {
    let result = ArgillaSettings::from_environment(
        &environment(&[("ARGILLA_API_KEY", "key")]),
        &ConstructorArguments::default(),
        None,
    );
    assert!(matches!(result, Err(SettingsError::MissingTransformation)));
}

#[rstest]
#[case::text(Json::Text("messages".to_owned()))]
#[case::list(Json::List(Vec::new()))]
#[case::boolean(Json::Bool(true))]
fn rejects_a_transformation_object_that_is_not_a_dictionary(#[case] value: Json) {
    assert!(matches!(
        build(&value),
        Err(SettingsError::TransformationNotDictionary)
    ));
}

#[rstest]
#[case::unknown_name(Json::Text("prompt".to_owned()), "prompt")]
#[case::number(Json::Number(serde_json::Number::from(7_u8)), "7")]
#[case::null(Json::Null, "null")]
fn rejects_payload_fields_outside_the_supported_set(#[case] value: Json, #[case] shown: &str) {
    let transformation = Json::Object(IndexMap::from([("input".to_owned(), value)]));
    let error = build(&transformation).err().expect("the value is rejected");
    assert!(matches!(error, SettingsError::UnsupportedField { .. }));
    assert!(error.to_string().contains(shown));
}

#[test]
fn accepts_messages_and_response_in_order() {
    let built = ArgillaSettings::from_environment(
        &environment(&[("ARGILLA_API_KEY", "key")]),
        &ConstructorArguments::default(),
        Some(&transformation_json()),
    )
    .expect("the settings are valid");
    assert_eq!(
        built
            .transformation
            .iter()
            .map(|(name, field)| (name.as_str(), *field))
            .collect::<Vec<_>>(),
        vec![("input", PayloadField::Messages)]
    );
}

#[test]
fn accepts_both_supported_fields() {
    let transformation = Json::Object(IndexMap::from([
        ("prompt".to_owned(), Json::Text("messages".to_owned())),
        ("completion".to_owned(), Json::Text("response".to_owned())),
    ]));
    let built = build(&transformation).expect("the settings are valid");
    assert_eq!(
        built.transformation.keys().collect::<Vec<_>>(),
        vec!["prompt", "completion"]
    );
    assert_eq!(built.transformation["completion"], PayloadField::Response);
}
