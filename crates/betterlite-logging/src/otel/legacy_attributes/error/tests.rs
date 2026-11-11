#![cfg(test)]

use super::LegacyAttributesError;

#[test]
fn missing_standard_logging_object_names_the_kwarg() {
    assert_eq!(
        LegacyAttributesError::MissingStandardLoggingObject.to_string(),
        "standard_logging_object not found in kwargs"
    );
}
