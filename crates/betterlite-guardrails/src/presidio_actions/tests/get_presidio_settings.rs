#![cfg(test)]

use betterlite_config::sections::guardrail_identity::enums::PiiEntityType;
use rstest::rstest;

use super::shared::request_data;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::presidio_settings_from_request;

#[rstest]
#[case::no_metadata("{}")]
#[case::null_metadata(r#"{"metadata":null}"#)]
#[case::missing_guardrail_config(r#"{"metadata":{"litellm_call_id":"x"}}"#)]
#[case::null_guardrail_config(r#"{"metadata":{"guardrail_config":null}}"#)]
#[case::empty_guardrail_config(r#"{"metadata":{"guardrail_config":{}}}"#)]
fn a_missing_or_empty_config_returns_none(#[case] json: &str) {
    let data = request_data(json);
    assert_eq!(presidio_settings_from_request(&data).unwrap(), None);
}

#[test]
fn language_and_entities_are_read_from_the_guardrail_config() {
    let data = request_data(
        r#"{"metadata":{"guardrail_config":{"language":"de","entities":["PERSON","EMAIL_ADDRESS"]}}}"#,
    );
    let config = presidio_settings_from_request(&data).unwrap().unwrap();
    assert_eq!(config.language.as_deref(), Some("de"));
    assert_eq!(
        config.entities,
        Some(vec![PiiEntityType::Person, PiiEntityType::EmailAddress])
    );
}

#[test]
fn explicit_nulls_inside_the_guardrail_config_stay_none() {
    let data =
        request_data(r#"{"metadata":{"guardrail_config":{"language":null,"entities":null}}}"#);
    let config = presidio_settings_from_request(&data).unwrap().unwrap();
    assert_eq!(config.language, None);
    assert_eq!(config.entities, None);
}

#[test]
fn keys_outside_the_per_request_model_are_ignored() {
    let data =
        request_data(r#"{"metadata":{"guardrail_config":{"language":"fr","future_option":42}}}"#);
    let config = presidio_settings_from_request(&data).unwrap().unwrap();
    assert_eq!(config.language.as_deref(), Some("fr"));
}

#[test]
fn a_metadata_value_that_is_not_an_object_is_an_error() {
    let data = request_data(r#"{"metadata":"text"}"#);
    let error = presidio_settings_from_request(&data).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::MetadataNotObject { .. }
    ));
    assert_eq!(error.status(), 400);
}

#[test]
fn a_guardrail_config_that_is_not_an_object_is_an_error() {
    let data = request_data(r#"{"metadata":{"guardrail_config":"text"}}"#);
    let error = presidio_settings_from_request(&data).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::GuardrailConfigNotObject { .. }
    ));
}

#[test]
fn a_language_of_the_wrong_type_names_the_field() {
    let data = request_data(r#"{"metadata":{"guardrail_config":{"language":5}}}"#);
    let error = presidio_settings_from_request(&data).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::GuardrailConfigField {
            field: "language",
            ..
        }
    ));
}

#[test]
fn an_unknown_entity_name_names_the_field() {
    let data = request_data(r#"{"metadata":{"guardrail_config":{"entities":["NOT_AN_ENTITY"]}}}"#);
    let error = presidio_settings_from_request(&data).unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::GuardrailConfigField {
            field: "entities",
            ..
        }
    ));
}
