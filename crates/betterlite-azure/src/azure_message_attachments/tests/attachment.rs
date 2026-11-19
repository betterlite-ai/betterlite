#![cfg(test)]

use betterlite_prompts::arguments::json::Object;

use crate::azure_message_attachments::legacy::{
    ParamsError, SUPPORTED_MESSAGE_PARAMS, map_create_message_params,
};

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn mapped(non_default: &str, existing: &str, drop_params: bool) -> Result<Object, ParamsError> {
    let mut optional = object(existing);
    map_create_message_params(&object(non_default), &mut optional, drop_params)?;
    Ok(optional)
}

pub(super) fn tools() {
    assert_eq!(
        SUPPORTED_MESSAGE_PARAMS,
        ["role", "content", "attachments", "metadata"]
    );
    assert_eq!(
        mapped(r#"{"role":"assistant","metadata":{"a":"b"}}"#, "{}", false).unwrap(),
        object(r#"{"role":"assistant","metadata":{"a":"b"}}"#)
    );
    assert_eq!(
        mapped(r#"{"content":"hello"}"#, "{}", false).unwrap(),
        object(r#"{"content":"hello"}"#)
    );
    assert_content_rejected(r#"{"content":5}"#);
    assert_content_rejected(r#"{"content":["hello"]}"#);
    assert_eq!(
        mapped(r#"{"unknown_key":5,"content":"x"}"#, "{}", false).unwrap(),
        object(r#"{"content":"x"}"#)
    );
    attachment_tools();
}

fn assert_content_rejected(non_default: &str) {
    let error = mapped(non_default, "{}", false).unwrap_err();
    assert!(matches!(error, ParamsError::ContentNotString));
    assert_eq!(error.status_code(), Some(400));
}

// The collected file ids are never assigned back: a valid list leaves optional_params untouched.
fn attachment_tools() {
    let non_default = r#"{"attachments":[{"file_id":"file-1"},{"file_id":"file-2","tools":[{"type":"file_search"}]}]}"#;
    assert_eq!(
        mapped(
            non_default,
            r#"{"attachments":["old"],"file_ids":["f0"]}"#,
            false
        )
        .unwrap(),
        object(r#"{"attachments":["old"],"file_ids":["f0"]}"#)
    );
    assert_eq!(mapped(non_default, "{}", false).unwrap(), object("{}"));
    assert_eq!(
        mapped(
            r#"{"attachments":[{"file_id":"f","tools":[{"type":"nope"}]}]}"#,
            "{}",
            false
        )
        .unwrap(),
        object("{}")
    );
}

pub(super) fn invalid_drop() {
    assert_eq!(
        mapped(
            r#"{"attachments":[{"tools":[]},{"file_id":"f"}]}"#,
            "{}",
            true
        )
        .unwrap(),
        object("{}")
    );
    assert_eq!(
        mapped(r#"{"attachments":[{"file_id":"file-1"}]}"#, "{}", true).unwrap(),
        object("{}")
    );
    let later = mapped(r#"{"attachments":[{"tools":[]},5]}"#, "{}", true).unwrap_err();
    assert!(matches!(later, ParamsError::MalformedAttachment));
    assert_eq!(later.status_code(), None);
}

pub(super) fn invalid_error() {
    let missing = mapped(r#"{"attachments":[{"tools":[]}]}"#, "{}", false).unwrap_err();
    assert!(matches!(missing, ParamsError::UnsupportedAttachment));
    assert_eq!(missing.status_code(), Some(400));
    let plain = mapped(r#"{"attachments":["plain"]}"#, "{}", false).unwrap_err();
    assert!(matches!(plain, ParamsError::UnsupportedAttachment));
    assert_eq!(plain.status_code(), Some(400));
    assert_not_list(r#""nope""#, false);
    assert_not_list(r#""nope""#, true);
    assert_not_list("5", true);
    assert_not_list("null", true);
    assert_not_list(r#"{"file_id":"f"}"#, true);
    let malformed = mapped(r#"{"attachments":[5]}"#, "{}", false).unwrap_err();
    assert!(matches!(malformed, ParamsError::MalformedAttachment));
    assert_eq!(malformed.status_code(), None);
    assert_eq!(
        mapped(r#"{"attachments":[{"file_id":null}]}"#, "{}", false).unwrap(),
        object("{}")
    );
    assert_eq!(
        mapped(r#"{"attachments":[{"file_id":"f"},{"x":1}]}"#, "{}", false)
            .unwrap_err()
            .status_code(),
        Some(400)
    );
    let ordered = mapped(
        r#"{"attachments":[{"file_id":"f"}],"content":3}"#,
        "{}",
        false,
    )
    .unwrap_err();
    assert!(matches!(ordered, ParamsError::ContentNotString));
}

fn assert_not_list(value: &str, drop_params: bool) {
    let non_default = format!(r#"{{"attachments":{value}}}"#);
    let error = mapped(&non_default, "{}", drop_params).unwrap_err();
    assert!(matches!(error, ParamsError::AttachmentsNotList));
    assert_eq!(error.status_code(), Some(400));
}
