use betterlite_guardrails::vigil_guard::vigil_guard_metadata::{CallIdSource, extract_call_id};

use crate::support::object;

struct Logging {
    call_id: Option<String>,
}

impl CallIdSource for Logging {
    fn litellm_call_id(&self) -> Option<&str> {
        self.call_id.as_deref()
    }
}

fn logging(call_id: Option<&str>) -> Logging {
    Logging {
        call_id: call_id.map(ToOwned::to_owned),
    }
}

#[test]
fn the_logging_object_wins() {
    let data = object(r#"{"litellm_call_id":"request"}"#);
    let source: &dyn CallIdSource = &logging(Some("logging"));
    assert_eq!(
        extract_call_id(&data, Some(source)),
        Some("logging".to_owned())
    );
}

#[test]
fn request_data_call_id_is_second() {
    let data = object(r#"{"litellm_call_id":"request"}"#);
    let source: &dyn CallIdSource = &logging(None);
    assert_eq!(
        extract_call_id(&data, Some(source)),
        Some("request".to_owned())
    );
}

#[test]
fn nested_metadata_is_last() {
    let data = object(r#"{"metadata":{"litellm_call_id":"nested"}}"#);
    let source: &dyn CallIdSource = &logging(Some(""));
    assert_eq!(
        extract_call_id(&data, Some(source)),
        Some("nested".to_owned())
    );
}

#[test]
fn empty_strings_are_skipped() {
    let data = object(r#"{"litellm_call_id":""}"#);
    let source: &dyn CallIdSource = &logging(Some(""));
    assert_eq!(extract_call_id(&data, Some(source)), None);
}

#[test]
fn non_string_values_are_skipped() {
    let data = object(r#"{"litellm_call_id":7,"metadata":{"litellm_call_id":"nested"}}"#);
    let source: &dyn CallIdSource = &logging(None);
    assert_eq!(
        extract_call_id(&data, Some(source)),
        Some("nested".to_owned())
    );
}

#[test]
fn litellm_metadata_is_not_a_call_id_source() {
    let data = object(r#"{"litellm_metadata":{"litellm_call_id":"deep"}}"#);
    let source: &dyn CallIdSource = &logging(None);
    assert_eq!(extract_call_id(&data, Some(source)), None);
}

#[test]
fn returns_none_without_a_call_id() {
    let data = object(r#"{"model":"gpt-4o"}"#);
    assert_eq!(extract_call_id(&data, None), None);
}
